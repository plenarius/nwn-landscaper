// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The hak every landscaper module shares: the whole texture library (`ls_<id>`), the cave entrance black and the tiles' shaders (made
//! from the game's own lit shaders, see [`shaders`]). An area's own hak keeps only what is its own (tiles, walkmeshes, baked textures,
//! minimaps, shadow maps); the shared one is listed after it.
//!
//! It lives beside the area haks in the NWN user `hak/` folder and is
//! rebuilt when the library changes (a stamp of its catalog and files).

use std::hash::{Hash, Hasher};
use std::path::Path;

use anyhow::{Context, Result};
use mg_core::{ResRef, ResType};
use mg_erf::ErfWriter;

use crate::paint;

/// The shared hak's name: the textures every generated area uses go in it once, not in each area's hak.
pub const HAK: &str = "ls_textures";
/// The cave entrance and void black.
pub const BLACK: &str = "ls_black";
/// A chasm wall's fade (an MTR's texture7, over the wall's own texture): white at its left, black from [`CHASM_DARK`] of the way
/// across. A wall's vertices take `u` from how far under the rim they are ([`chasm_fade_u`]).
pub const CHASM_FADE: &str = "ls_chasmfade";
/// How far down a chasm (a fraction of its depth) its walls are black: below this only the fog shows anything.
pub const CHASM_DARK: f32 = 0.65;
/// The fade texture's width (pixels).
const FADE_WIDTH: u32 = 64;

/// How bright a chasm wall is `t` of the way down (0 at the rim, 1 at the floor), as the game multiplies its colour: falling
/// fast near the rim, black from [`CHASM_DARK`] down. The editor's `chasmFade` (js/view.js) is the same curve.
pub fn chasm_fade(t: f32) -> f32 {
    (1.0 - t / CHASM_DARK).clamp(0.0, 1.0).powf(1.6)
}

/// The fade texture's `u` for a vertex `t` of the way down: on its pixels' centres, so neither end blends with the other.
pub fn chasm_fade_u(t: f32) -> f32 {
    (0.5 + t.clamp(0.0, 1.0) * (FADE_WIDTH - 1) as f32) / FADE_WIDTH as f32
}

/// Builds `<hak_dir>/ls_textures.hak` unless it is up to date; whether it was built.
pub fn ensure(hak_dir: &Path) -> Result<bool> {
    let path = hak_dir.join(format!("{HAK}.hak"));
    let stamp_path = paint::data_dir().join(format!("{HAK}.stamp"));
    let stamp = stamp()?;
    if path.exists() && std::fs::read_to_string(&stamp_path).is_ok_and(|s| s.trim() == stamp) {
        return Ok(false);
    }
    let mut hak = ErfWriter::new(*b"HAK ");
    let mut owned: Vec<(ResRef, ResType, Vec<u8>)> = Vec::new();
    let ids = library_ids()?;
    let library = paint::load_library(&ids)?;
    for id in &ids {
        owned.push((ResRef::from_str(&paint::resref(id))?, ResType::DDS, paint::dds(&library[id].image)));
    }
    owned.push((ResRef::from_str(BLACK)?, ResType::DDS, paint::dds(&paint::picture(4, 4, |_, _| [0.0, 0.0, 0.0]))));
    let fade = paint::picture(FADE_WIDTH, 4, |x, _| [chasm_fade(x as f32 / (FADE_WIDTH - 1) as f32); 3]);
    owned.push((ResRef::from_str(CHASM_FADE)?, ResType::DDS, paint::dds(&fade)));
    owned.push((ResRef::from_str(CHASM_FADE)?, ResType::TXI, b"clamp 3\r\n".to_vec()));
    for (name, text) in shaders()? {
        owned.push((ResRef::from_str(name)?, ResType::SHD, text.into_bytes()));
    }
    for (r, t, d) in &owned {
        hak.add(*r, *t, d.as_slice())?;
    }
    std::fs::create_dir_all(hak_dir)?;
    crate::atomic::write(&path, &hak.to_bytes()?)?;
    std::fs::write(&stamp_path, &stamp)?;
    println!("shared hak: {} ({} library textures)", path.display(), ids.len());
    Ok(true)
}

/// Every texture id in the library's catalog.
fn library_ids() -> Result<Vec<String>> {
    let catalog: serde_json::Value = serde_json::from_slice(&crate::atomic::read(&paint::texture_dir().join("catalog.json"))?)?;
    Ok(catalog["textures"]
        .as_array()
        .context("catalog.json: textures")?
        .iter()
        .filter_map(|t| t["id"].as_str().map(str::to_string))
        .collect())
}

/// What the shared hak is built from: the catalog, each texture file's size and time, the shaders and this format.
fn stamp() -> Result<String> {
    let dir = paint::texture_dir();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let catalog = crate::atomic::read(&dir.join("catalog.json"))
        .with_context(|| format!("no texture library in {} (run `landscaper textures init`)", dir.display()))?;
    catalog.hash(&mut h);
    let parsed: serde_json::Value = serde_json::from_slice(&catalog)?;
    for t in parsed["textures"].as_array().into_iter().flatten() {
        if let Some(file) = t["render"].as_str().or(t["file"].as_str())
            && let Ok(m) = std::fs::metadata(dir.join(file))
        {
            m.len().hash(&mut h);
            m.modified().ok().hash(&mut h);
        }
    }
    shaders()?.hash(&mut h);
    // (2: the chasm fade joined the black; 3: the triplanar shader; 4: the edge scenery's fade)
    4u32.hash(&mut h);
    Ok(format!("{:016x}", h.finish()))
}

/// The tiles' shaders, `ls_lm_vs`, `ls_lm_fs` and `ls_tp_fs`, and the lighting they share, `ls_lighting`: the game's stock lit pair
/// (`vslit`, `fslit`) and its lighting (`inc_lighting`) read from the install and changed here, so no copy of Beamdog's shaders is in
/// this repository or in a release of it.
///
/// The tile's baked shadow map (MTR texture7) shades the area's own light only, its sun or moon (ambient and diffuse): local
/// lights (tile lights, placeables' lights, a spell's glow) and a mesh's self-illumination light shaded ground as they light any
/// other. Where nothing else lights the ground, this is the stock colour times the shadow map, as before.
///
/// - `ls_lighting`: inc_lighting with the area's ambient and diffuse colours multiplied by `lsAreaShade` (1 unless a shader sets
///   it), and steep ground given some of the sun or moon bounced off the ground ([`BOUNCE`]). Included before `inc_standard`, it
///   takes stock's place (same include guard).
/// - `ls_lm_vs`: vslit, passing the second texture coordinates (MDL tverts1) on to the fragment shader. Lit per vertex (the game's
///   "Minimal" shader setting), it lights each vertex a second time without the area's light, for the fragment shader to blend.
/// - `ls_lm_fs`: fslit, its area light shaded by the shadow map at those coordinates: lit per fragment, through `lsAreaShade`;
///   lit per vertex, by blending the vertex shader's two lights. Without a texture7 the surface is drawn as fslit draws it.
/// - `ls_tp_fs`: `ls_lm_fs` with its texture drawn from three sides by the slope ([`crate::triplanar`]), for library textures:
///   see [`patch_triplanar`].
/// - `ls_fd_fs`: `ls_lm_fs` fading to the fog colour past the area's edge, for the edge scenery over a halo; `ls_fdt_fs` the same
///   for the top of a tree's cards, which also fades out when seen edge-on: see [`patch_fade`].
///
/// An install without them (the dedicated server's download has no shaders) or with a different shape of them is an error that
/// says so: without them the area would draw wrong, not only without shadows.
pub fn shaders() -> Result<[(&'static str, String); 6]> {
    let game = crate::models::game()?;
    // The browser build with no game folder given: placeholders, so the rest of the export still downloads. It draws wrong in the
    // game until exported again with the folder (docs/DESIGN-web-demo.md, question 5).
    if cfg!(feature = "web") && game.resman.get_named("fslit", ResType::SHD).is_err() {
        return Ok([LIGHTING, "ls_lm_vs", "ls_tp_fs", "ls_lm_fs", "ls_fd_fs", "ls_fdt_fs"].map(|n| (n, STUB_SHADER.to_string())));
    }
    let stock = |name: &str| -> Result<String> {
        let data = game.resman.get_named(name, ResType::SHD).map_err(|e| {
            anyhow::anyhow!(
                "the game's {name}.shd is not in the install ({e}): the shared hak's tile shaders are made from it. Point NWN_ROOT \
                 (or --root) at the game itself; the dedicated server's download has no shaders"
            )
        })?;
        Ok(String::from_utf8_lossy(&data).into_owned())
    };
    let fs = patch_fslit(&stock("fslit")?)?;
    Ok([
        (LIGHTING, patch_lighting(&stock("inc_lighting")?)?),
        ("ls_lm_vs", patch_vslit(&stock("vslit")?)?),
        ("ls_tp_fs", patch_triplanar(&fs)?),
        ("ls_fd_fs", patch_fade(&fs, false)?),
        ("ls_fdt_fs", patch_fade(&fs, true)?),
        ("ls_lm_fs", fs),
    ])
}

/// What stands in for the tile shaders when the browser build has no game folder to make them from (see [`shaders`]).
const STUB_SHADER: &str = "// placeholder: the Landscaper makes this shader from the game's own when it can read the game's folder\n";

/// The tiles' lighting, `inc_lighting` changed (see [`shaders`]).
const LIGHTING: &str = "ls_lighting";

/// Replaces the one line of `text` that reads `anchor` (spaces around it aside) with `with` (its indent kept). A stock shader with
/// none or several is not the shape the change is written for: an error, not a guess.
fn replace_line(text: &str, name: &str, anchor: &str, with: &[&str]) -> Result<String> {
    let lines: Vec<&str> = text.lines().collect();
    let at: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.trim() == anchor).map(|(i, _)| i).collect();
    let [i] = at[..] else {
        anyhow::bail!(
            "the game's {name}.shd has {} lines `{anchor}`, not one: the Landscaper's shadow shader is made by changing that line. \
             A game update may have changed the shader; please report it with the game's version",
            at.len()
        );
    };
    let indent = &lines[i][..lines[i].len() - lines[i].trim_start().len()];
    let mut out: Vec<String> = lines[..i].iter().map(|l| (*l).to_string()).collect();
    out.extend(with.iter().map(|w| if w.is_empty() || w.starts_with('#') { (*w).to_string() } else { format!("{indent}{w}") }));
    out.extend(lines[i + 1..].iter().map(|l| (*l).to_string()));
    Ok(out.join("\n") + "\n")
}

/// The first line of a made shader: where it comes from.
fn made_from(stock: &str) -> String {
    format!("// Made by the Landscaper from the game's {stock}.shd at run time (shared.rs, `shaders`): Beamdog's shader, changed.\n")
}

/// inc_lighting, the area's ambient and diffuse light multiplied by `lsAreaShade` (see [`shaders`]).
fn patch_lighting(stock: &str) -> Result<String> {
    let name = "inc_lighting";
    let s = replace_line(
        stock,
        name,
        "uniform vec3 lightAreaDiffuseDirection;",
        &[
            "uniform vec3 lightAreaDiffuseDirection;",
            "// The Landscaper's baked shade on the area's sun or moon, set by its tile shaders.",
            "highp vec3 lsAreaShade = vec3(1.0);",
        ],
    )?;
    let bounce = format!(
        "AmbientLight = (lightAreaAmbient + lightAreaDiffuse * ({:.2} * (1.0 - normalize(mat3(m_view_inv) * vNormalThis).z))) * lsAreaShade;",
        BOUNCE / 2.0
    );
    let s = replace_line(
        &s,
        name,
        "AmbientLight = lightAreaAmbient;",
        &[
            "// The area's ambient light, and the sun's or moon's bounced off the ground onto steep ground (the Landscaper's shared.rs).",
            &bounce,
        ],
    )?;
    let s = replace_line(
        &s,
        name,
        "ComputeDiffuseLighting(lightAreaDiffuse, lightAreaDiffuseDirection, vNormalThis, 1.0, false);",
        &["ComputeDiffuseLighting(lightAreaDiffuse * lsAreaShade, lightAreaDiffuseDirection, vNormalThis, 1.0, false);"],
    )?;
    Ok(made_from(name) + &s)
}

/// How much of the area's sun or moon a vertical face takes back from the ground (half of it; none on flat ground). The game
/// lights ground turned from its fixed sun with the ambient light alone, a dim blue by day, so a cliff on that side came out near
/// black; real ones are lit by the sunlit ground below them.
pub const BOUNCE: f32 = 0.5;

/// Includes [`LIGHTING`] in place of the stock lighting, which `inc_standard` then skips.
fn include_lighting(text: &str, name: &str) -> Result<String> {
    let include = format!("#include \"{LIGHTING}\"");
    replace_line(text, name, "#include \"inc_standard\"", &[&include, "#include \"inc_standard\""])
}

/// Lit per vertex: `inc_standard` lights in the vertex shader and hands the fragment shader `VertexColor`.
const PER_VERTEX: &str = "#if LIGHTING == 1 && FRAGMENT_LIGHTING != 1 && FRAGMENT_NORMAL != 1";

/// vslit, passing the second texture coordinates on and, lit per vertex, the light without the area's (see [`shaders`]).
fn patch_vslit(stock: &str) -> Result<String> {
    let s = include_lighting(stock, "vslit")?;
    let s = replace_line(
        &s,
        "vslit",
        "#include \"inc_standard\"",
        &[
            "#include \"inc_standard\"",
            "",
            "attribute vec2 vTcIn1;",
            "attribute vec2 vTcIn2;",
            "varying vec2 vShadowUV;",
            "varying vec2 vLsBlend;",
            "varying vec3 vLsLocalLight;",
        ],
    )?;
    let s = replace_line(
        &s,
        "vslit",
        "ApplyStandardShader();",
        &[
            "ApplyStandardShader();",
            "vShadowUV = vTcIn1;",
            "vLsBlend = vTcIn2;",
            PER_VERTEX,
            "// The vertex lit again without the area's sun or moon: the fragment shader blends the two by the baked shade.",
            "vec4 lsLocal = COLOR_WHITE;",
            "lsAreaShade = vec3(0.0);",
            "ComputeLighting(lsLocal, vVertexNormal);",
            "vLsLocalLight = lsLocal.rgb;",
            "#else",
            "vLsLocalLight = vec3(0.0);",
            "#endif",
        ],
    )?;
    Ok(made_from("vslit") + &s)
}

/// fslit, its area light shaded by the baked shadow map (see [`shaders`]).
fn patch_fslit(stock: &str) -> Result<String> {
    let s = include_lighting(stock, "fslit")?;
    let s = replace_line(
        &s,
        "fslit",
        "#include \"inc_standard\"",
        &[
            "#include \"inc_standard\"",
            "",
            "uniform sampler2D texUnit7;",
            "uniform lowp int texture7Bound;",
            "varying vec2 vShadowUV;",
            "varying vec3 vLsLocalLight;",
        ],
    )?;
    let s = replace_line(
        &s,
        "fslit",
        "ApplyStandardShader();",
        &[
            "// The Landscaper's baked shade, on the area's sun or moon only (its shared.rs). In the light's colour space, so where",
            "// nothing else lights the ground the colour comes out the shadow map times the stock one.",
            "if (texture7Bound != 0)",
            "{",
            "    vec3 lsShade = ApplyColorSpace(texture2D(texUnit7, vShadowUV).rgb);",
            PER_VERTEX,
            "    // VertexColor is the light with the area's at full, vLsLocalLight without it; the second is never the brighter.",
            "    FragmentColor.rgb *= mix(vLsLocalLight, VertexColor.rgb, lsShade) / max(VertexColor.rgb, vec3(0.0001));",
            "#else",
            "    lsAreaShade = lsShade;",
            "#endif",
            "}",
            "ApplyStandardShader();",
        ],
    )?;
    Ok(made_from("fslit") + &s)
}

/// `ls_lm_fs` (made by [`patch_fslit`]) drawing its texture from three sides by the slope ([`crate::triplanar`]): after the stock
/// shader has read its inputs, the texture's colour is sampled again at the world position (from the view position and the
/// camera's inverse view matrix, as the game's own keyhole shader finds it) along x, along y and from above, weighted by the vertex
/// normal turned back into the world, and takes the place of the colour read at the mesh's texture coordinates. Lighting, the
/// shadow map and fog then go on as before. (The stock read is wasted; the specular light's colour, which the stock shader takes
/// from it, comes out of the same texture and differs only where the two readings do.)
fn patch_triplanar(ls_lm_fs: &str) -> Result<String> {
    use crate::triplanar::SHARPNESS;
    // (Metres a repeat: the material's lsMetres, the texture's own; paint::TEXTURE_METRES unless its catalog entry says more.)
    let pos = "highp vec3 lsP = (m_view_inv * vec4(vPosView, 1.0)).xyz / lsMetres;";
    let weights = format!("vec3 lsW = pow(abs(lsN), vec3({:.1}));", f64::from(SHARPNESS));
    let s = replace_line(
        ls_lm_fs,
        "fslit",
        "varying vec2 vShadowUV;",
        &["varying vec2 vShadowUV;", "varying vec2 vLsBlend;", "uniform float lsMetres;"],
    )?;
    let s = replace_line(
        &s,
        "fslit",
        "SetupStandardShaderInputs();",
        &[
            "SetupStandardShaderInputs();",
            "// The Landscaper's triplanar texture (its triplanar.rs): along x, along y and from above, by the slope.",
            "if (texture0Bound != 0)",
            "{",
            &format!("    {pos}"),
            "    // Blended by the ground's own slope from the mesh's tverts2 (each plus 2; triplanar::blend_normal), else the vertex normal.",
            "    vec3 lsN = normalize(mat3(m_view_inv) * vVertexNormal);",
            "    if (vLsBlend.x > 0.5)",
            "    {",
            "        vec2 lsB = vLsBlend - 2.0;",
            "        lsN = vec3(lsB, sqrt(max(0.0, 1.0 - dot(lsB, lsB))));",
            "    }",
            &format!("    {weights}"),
            "    lsW /= lsW.x + lsW.y + lsW.z;",
            "    vec4 lsTex = texture2D(texUnit0, lsP.yz) * lsW.x + texture2D(texUnit0, lsP.xz) * lsW.y + texture2D(texUnit0, lsP.xy) * lsW.z;",
            "    FragmentColor.rgb = ApplyColorSpace(lsTex.rgb);",
            "}",
        ],
    )?;
    Ok(s)
}

/// The fade past the area's edge (m): from, to. The edge scenery reaches 50 m, as far as the client's scene covers.
pub const FADE: [f32; 2] = [25.0, 50.0];

/// `ls_lm_fs` (made by [`patch_fslit`]) fading to the fog colour past the area's edge, from [`FADE`]'s first distance to its
/// second, measured from the nearest point of the area (its size from the material's `lsAreaW` and `lsAreaH`): the edge
/// scenery's end meets the background instead of stopping. `top`: also fading out when seen edge-on (a tree card lying flat over
/// the crown, which from the side reads as a brim).
fn patch_fade(ls_lm_fs: &str, top: bool) -> Result<String> {
    let s = replace_line(
        ls_lm_fs,
        "fslit",
        "varying vec3 vLsLocalLight;",
        &["varying vec3 vLsLocalLight;", "uniform float lsAreaW;", "uniform float lsAreaH;"],
    )?;
    let [from, to] = FADE;
    let mut with = vec![
        "// The Landscaper's edge scenery (its overhang.rs): towards the fog colour past the area's edge.".to_string(),
        "{".to_string(),
        "    highp vec3 lsWP = (m_view_inv * vec4(vPosView, 1.0)).xyz;".to_string(),
        "    highp vec2 lsOut = max(max(-lsWP.xy, lsWP.xy - vec2(lsAreaW, lsAreaH)), vec2(0.0));".to_string(),
        format!("    FragmentColor.rgb = mix(FragmentColor.rgb, fogColor.rgb, smoothstep({from:.1}, {to:.1}, length(lsOut)));"),
        "}".to_string(),
    ];
    if top {
        with.extend(
            [
                "// Seen from the side, the top card shows only from about 25 degrees down, fully from 45.",
                "FragmentColor.a *= smoothstep(0.42, 0.70, abs(dot(normalize(vVertexNormal), normalize(-vPosView))));",
                "if (FragmentColor.a < 0.04) discard;",
            ]
            .map(str::to_string),
        );
    }
    with.push("gl_FragColor = FragmentColor;".to_string());
    let with: Vec<&str> = with.iter().map(String::as_str).collect();
    replace_line(&s, "fslit", "gl_FragColor = FragmentColor;", &with)
}

/// `landscaper shaders <out>`: writes the three shaders as [`shaders`] makes them, to compare them with what a release had.
pub fn write_shaders(out: &Path) -> Result<()> {
    std::fs::create_dir_all(out)?;
    for (name, text) in shaders()? {
        let path = out.join(format!("{name}.shd"));
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        println!("{}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The code lines of a shader: no comments or blank lines, spaces at the ends trimmed.
    fn code(text: &str) -> Vec<String> {
        text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//")).map(str::to_string).collect()
    }

    /// Made-up stand-ins for the stock shaders: other lines around the ones the patches look for (the `inc_standard` include,
    /// `SetupStandardShaderInputs();`, `ApplyStandardShader();`, the area's light in the lighting), indented as the game's are.
    const VSLIT: &str = "#define MADE_UP 1\n#include \"inc_standard\"\n\nvoid main ()\n{\n    Before();\n    ApplyStandardShader();\n}\n";
    const FSLIT: &str = "#define MADE_UP 2\n#define FOG 1\n#include \"inc_standard\"\n\nvoid main ()\n{\n    Before();\n    \
        SetupStandardShaderInputs();\n    ApplyStandardShader();\n    gl_FragColor = FragmentColor;\n}\n";
    const LIGHTING_STOCK: &str = "#ifndef INC_LIGHTING\nuniform vec3 lightAreaAmbient;\nuniform vec3 lightAreaDiffuse;\n\
        uniform vec3 lightAreaDiffuseDirection;\nvoid ComputeLighting(inout vec4 ColorIO, vec3 vNormalThis)\n{\n    \
        AmbientLight = lightAreaAmbient;\n\tComputeDiffuseLighting(lightAreaDiffuse, lightAreaDiffuseDirection, vNormalThis, 1.0, \
        false);\n    for (int i = 0; i < numLights; i++) {}\n}\n";

    #[test]
    fn the_lighting_shades_the_area_s_light_and_not_the_others() {
        let lighting = patch_lighting(LIGHTING_STOCK).unwrap();
        let want = [
            "#ifndef INC_LIGHTING",
            "uniform vec3 lightAreaAmbient;",
            "uniform vec3 lightAreaDiffuse;",
            "uniform vec3 lightAreaDiffuseDirection;",
            "highp vec3 lsAreaShade = vec3(1.0);",
            "void ComputeLighting(inout vec4 ColorIO, vec3 vNormalThis)",
            "{",
            "AmbientLight = (lightAreaAmbient + lightAreaDiffuse * (0.25 * (1.0 - normalize(mat3(m_view_inv) * vNormalThis).z))) * lsAreaShade;",
            "ComputeDiffuseLighting(lightAreaDiffuse * lsAreaShade, lightAreaDiffuseDirection, vNormalThis, 1.0, false);",
            "for (int i = 0; i < numLights; i++) {}",
            "}",
        ];
        assert_eq!(code(&lighting), want);
        assert!(lighting.contains("\n\tComputeDiffuseLighting(lightAreaDiffuse * lsAreaShade,"), "the indent is kept");
    }

    #[test]
    fn the_shadow_shaders_shade_the_area_light_and_keep_the_rest() {
        let vs = [
            "#define MADE_UP 1",
            "#include \"ls_lighting\"",
            "#include \"inc_standard\"",
            "attribute vec2 vTcIn1;",
            "attribute vec2 vTcIn2;",
            "varying vec2 vShadowUV;",
            "varying vec2 vLsBlend;",
            "varying vec3 vLsLocalLight;",
            "void main ()",
            "{",
            "Before();",
            "ApplyStandardShader();",
            "vShadowUV = vTcIn1;",
            "vLsBlend = vTcIn2;",
            PER_VERTEX,
            "vec4 lsLocal = COLOR_WHITE;",
            "lsAreaShade = vec3(0.0);",
            "ComputeLighting(lsLocal, vVertexNormal);",
            "vLsLocalLight = lsLocal.rgb;",
            "#else",
            "vLsLocalLight = vec3(0.0);",
            "#endif",
            "}",
        ];
        assert_eq!(code(&patch_vslit(VSLIT).unwrap()), vs);
        let fs = [
            "#define MADE_UP 2",
            "#define FOG 1",
            "#include \"ls_lighting\"",
            "#include \"inc_standard\"",
            "uniform sampler2D texUnit7;",
            "uniform lowp int texture7Bound;",
            "varying vec2 vShadowUV;",
            "varying vec3 vLsLocalLight;",
            "void main ()",
            "{",
            "Before();",
            "SetupStandardShaderInputs();",
            "if (texture7Bound != 0)",
            "{",
            "vec3 lsShade = ApplyColorSpace(texture2D(texUnit7, vShadowUV).rgb);",
            PER_VERTEX,
            "FragmentColor.rgb *= mix(vLsLocalLight, VertexColor.rgb, lsShade) / max(VertexColor.rgb, vec3(0.0001));",
            "#else",
            "lsAreaShade = lsShade;",
            "#endif",
            "}",
            "ApplyStandardShader();",
            "gl_FragColor = FragmentColor;",
            "}",
        ];
        assert_eq!(code(&patch_fslit(FSLIT).unwrap()), fs);
        assert!(patch_fslit(FSLIT).unwrap().contains("\n    ApplyStandardShader();\n"), "the indent is kept");
        assert!(patch_fslit(FSLIT).unwrap().contains("\n#else\n"), "preprocessor lines are not indented");
    }

    #[test]
    fn the_triplanar_shader_replaces_the_texture_after_the_stock_read_and_before_the_light() {
        let tp = patch_triplanar(&patch_fslit(FSLIT).unwrap()).unwrap();
        let lines = code(&tp);
        let at = |l: &str| lines.iter().position(|x| x == l).unwrap_or_else(|| panic!("no `{l}` in\n{tp}"));
        let read = at("SetupStandardShaderInputs();");
        let replaced = at("FragmentColor.rgb = ApplyColorSpace(lsTex.rgb);");
        assert!(read < replaced && replaced < at("lsAreaShade = lsShade;") && replaced < at("ApplyStandardShader();"));
        // The projections and weights triplanar.rs has: a repeat every lsMetres (the material's, the texture's own metres), along
        // x (y, z), along y (x, z), from above (x, y), power 4.
        assert!(tp.contains("uniform float lsMetres;") && tp.contains(".xyz / lsMetres;") && tp.contains("vec3(4.0)"), "{tp}");
        assert!(tp.contains("texUnit0, lsP.yz) * lsW.x") && tp.contains("texUnit0, lsP.xz) * lsW.y") && tp.contains("lsP.xy) * lsW.z"));
        // Blended by the mesh's tverts2 where it has them, else by the vertex normal.
        assert!(lines.contains(&"varying vec2 vLsBlend;".to_string()) && tp.contains("if (vLsBlend.x > 0.5)"), "{tp}");
    }

    #[test]
    fn a_stock_shader_of_another_shape_is_an_error() {
        let e = patch_fslit(&FSLIT.replace("    ApplyStandardShader();\n", "")).unwrap_err().to_string();
        assert!(e.contains("fslit.shd has 0 lines `ApplyStandardShader();`"), "{e}");
        let e = patch_lighting(&LIGHTING_STOCK.replace("lightAreaAmbient;\n\t", "lightAreaAmbient * 2.0;\n\t")).unwrap_err().to_string();
        assert!(e.contains("inc_lighting.shd has 0 lines `AmbientLight = lightAreaAmbient;`"), "{e}");
        let twice = VSLIT.replace("ApplyStandardShader();", "ApplyStandardShader();\n    ApplyStandardShader();");
        assert!(patch_vslit(&twice).unwrap_err().to_string().contains("has 2 lines"));
    }
}
