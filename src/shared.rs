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
    // (2: the chasm fade joined the black; 3: the triplanar shader)
    3u32.hash(&mut h);
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
///   it). Included before `inc_standard`, it takes stock's place (same include guard).
/// - `ls_lm_vs`: vslit, passing the second texture coordinates (MDL tverts1) on to the fragment shader. Lit per vertex (the game's
///   "Minimal" shader setting), it lights each vertex a second time without the area's light, for the fragment shader to blend.
/// - `ls_lm_fs`: fslit, its area light shaded by the shadow map at those coordinates: lit per fragment, through `lsAreaShade`;
///   lit per vertex, by blending the vertex shader's two lights. Without a texture7 the surface is drawn as fslit draws it.
/// - `ls_tp_fs`: `ls_lm_fs` with its texture drawn from three sides by the slope ([`crate::triplanar`]), for library textures:
///   see [`patch_triplanar`].
///
/// An install without them (the dedicated server's download has no shaders) or with a different shape of them is an error that
/// says so: without them the area would draw wrong, not only without shadows.
pub fn shaders() -> Result<[(&'static str, String); 4]> {
    let game = crate::models::game()?;
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
        ("ls_lm_fs", fs),
    ])
}

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
    let s = replace_line(&s, name, "AmbientLight = lightAreaAmbient;", &["AmbientLight = lightAreaAmbient * lsAreaShade;"])?;
    let s = replace_line(
        &s,
        name,
        "ComputeDiffuseLighting(lightAreaDiffuse, lightAreaDiffuseDirection, vNormalThis, 1.0, false);",
        &["ComputeDiffuseLighting(lightAreaDiffuse * lsAreaShade, lightAreaDiffuseDirection, vNormalThis, 1.0, false);"],
    )?;
    Ok(made_from(name) + &s)
}

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
        &["#include \"inc_standard\"", "", "attribute vec2 vTcIn1;", "varying vec2 vShadowUV;", "varying vec3 vLsLocalLight;"],
    )?;
    let s = replace_line(
        &s,
        "vslit",
        "ApplyStandardShader();",
        &[
            "ApplyStandardShader();",
            "vShadowUV = vTcIn1;",
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
    let metres = paint::TEXTURE_METRES;
    let pos = format!("highp vec3 lsP = (m_view_inv * vec4(vPosView, 1.0)).xyz / {metres:.1};");
    let weights = format!("vec3 lsW = pow(abs(normalize(mat3(m_view_inv) * vVertexNormal)), vec3({:.1}));", f64::from(SHARPNESS));
    let s = replace_line(
        ls_lm_fs,
        "fslit",
        "SetupStandardShaderInputs();",
        &[
            "SetupStandardShaderInputs();",
            "// The Landscaper's triplanar texture (its triplanar.rs): along x, along y and from above, by the slope.",
            "if (texture0Bound != 0)",
            "{",
            &format!("    {pos}"),
            &format!("    {weights}"),
            "    lsW /= lsW.x + lsW.y + lsW.z;",
            "    vec4 lsTex = texture2D(texUnit0, lsP.yz) * lsW.x + texture2D(texUnit0, lsP.xz) * lsW.y + texture2D(texUnit0, lsP.xy) * lsW.z;",
            "    FragmentColor.rgb = ApplyColorSpace(lsTex.rgb);",
            "}",
        ],
    )?;
    Ok(s)
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
            "AmbientLight = lightAreaAmbient * lsAreaShade;",
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
            "varying vec2 vShadowUV;",
            "varying vec3 vLsLocalLight;",
            "void main ()",
            "{",
            "Before();",
            "ApplyStandardShader();",
            "vShadowUV = vTcIn1;",
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
        // The projections and weights triplanar.rs has: 4 m repeats, along x (y, z), along y (x, z), from above (x, y), power 4.
        assert!(tp.contains("/ 4.0;") && tp.contains("vec3(4.0)"), "{tp}");
        assert!(tp.contains("texUnit0, lsP.yz) * lsW.x") && tp.contains("texUnit0, lsP.xz) * lsW.y") && tp.contains("lsP.xy) * lsW.z"));
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
