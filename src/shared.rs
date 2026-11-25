// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The hak every landscaper module shares: the whole texture library (`ls_<id>`), the cave entrance black and the shadow shaders (made
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
    let catalog: serde_json::Value = serde_json::from_slice(&std::fs::read(paint::texture_dir().join("catalog.json"))?)?;
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
    let catalog = std::fs::read(dir.join("catalog.json"))
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
    // (2: the chasm fade joined the black)
    2u32.hash(&mut h);
    Ok(format!("{:016x}", h.finish()))
}

/// The shadow shaders, `ls_lm_vs` and `ls_lm_fs`: the game's stock lit pair (`vslit`, `fslit`) read from the install and changed
/// here, so no copy of Beamdog's shaders is in this repository or in a release of it.
///
/// - `ls_lm_vs`: vslit, passing the second texture coordinates (MDL tverts1) on to the fragment shader.
/// - `ls_lm_fs`: fslit, its colour darkened by the tile's baked shadow map (MTR texture7, at those coordinates) and then fogged.
///   Fog comes last so baked shadows fade into the distance like everything else; the stock shader would fog first, so `FOG` is
///   off and `ApplyFog` is called after the shadow. Without a texture7 the surface is drawn as fslit draws it.
///
/// An install without the two (the dedicated server's download has no shaders) or with a different shape of them is an error that
/// says so: without them the area would draw wrong, not only without shadows.
pub fn shaders() -> Result<[(&'static str, String); 2]> {
    let game = crate::models::game()?;
    let stock = |name: &str| -> Result<String> {
        let data = game.resman.get_named(name, ResType::SHD).map_err(|e| {
            anyhow::anyhow!(
                "the game's {name}.shd is not in the install ({e}): the shared hak's shadow shaders are made from it. Point NWN_ROOT \
                 (or --root) at the game itself; the dedicated server's download has no shaders"
            )
        })?;
        Ok(String::from_utf8_lossy(&data).into_owned())
    };
    Ok([("ls_lm_vs", patch_vslit(&stock("vslit")?)?), ("ls_lm_fs", patch_fslit(&stock("fslit")?)?)])
}

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

/// vslit, passing the second texture coordinates on to the fragment shader (see [`shaders`]).
fn patch_vslit(stock: &str) -> Result<String> {
    let s = replace_line(
        stock,
        "vslit",
        "#include \"inc_standard\"",
        &["#include \"inc_standard\"", "", "attribute vec2 vTcIn1;", "varying vec2 vShadowUV;"],
    )?;
    let s = replace_line(&s, "vslit", "ApplyStandardShader();", &["ApplyStandardShader();", "vShadowUV = vTcIn1;"])?;
    Ok(made_from("vslit") + &s)
}

/// fslit, darkened by the baked shadow map and then fogged (see [`shaders`]).
fn patch_fslit(stock: &str) -> Result<String> {
    let s = replace_line(stock, "fslit", "#define FOG 1", &["#define FOG 0"])?;
    let s = replace_line(
        &s,
        "fslit",
        "#include \"inc_standard\"",
        &[
            "// inc_standard includes inc_fog only with FOG 1.",
            "#include \"inc_fog\"",
            "#include \"inc_standard\"",
            "",
            "uniform sampler2D texUnit7;",
            "uniform lowp int texture7Bound;",
            "varying vec2 vShadowUV;",
        ],
    )?;
    let s = replace_line(
        &s,
        "fslit",
        "gl_FragColor = FragmentColor;",
        &[
            "if (texture7Bound != 0)",
            "    FragmentColor.rgb *= texture2D(texUnit7, vShadowUV).rgb;",
            "ApplyFog(FragmentColor);",
            "gl_FragColor = FragmentColor;",
        ],
    )?;
    Ok(made_from("fslit") + &s)
}

/// `landscaper shaders <out>`: writes the two shaders as [`shaders`] makes them, to compare them with what a release had.
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

    /// Made-up stand-ins for the stock pair: other lines around the ones the patch looks for (`#define FOG 1`, the
    /// `inc_standard` include, `ApplyStandardShader();` and `gl_FragColor = FragmentColor;`), indented as the game's are.
    const VSLIT: &str = "#define MADE_UP 1\n#include \"inc_standard\"\n\nvoid main ()\n{\n    Before();\n    ApplyStandardShader();\n}\n";
    const FSLIT: &str = "#define MADE_UP 2\n#define FOG 1\n#include \"inc_standard\"\n\nvoid main ()\n{\n    Before();\n    \
        gl_FragColor = FragmentColor;\n}\n";

    #[test]
    fn the_shadow_shaders_add_the_shadow_map_and_keep_the_rest() {
        let vs = [
            "#define MADE_UP 1",
            "#include \"inc_standard\"",
            "attribute vec2 vTcIn1;",
            "varying vec2 vShadowUV;",
            "void main ()",
            "{",
            "Before();",
            "ApplyStandardShader();",
            "vShadowUV = vTcIn1;",
            "}",
        ];
        assert_eq!(code(&patch_vslit(VSLIT).unwrap()), vs);
        let fs = [
            "#define MADE_UP 2",
            "#define FOG 0",
            "#include \"inc_fog\"",
            "#include \"inc_standard\"",
            "uniform sampler2D texUnit7;",
            "uniform lowp int texture7Bound;",
            "varying vec2 vShadowUV;",
            "void main ()",
            "{",
            "Before();",
            "if (texture7Bound != 0)",
            "FragmentColor.rgb *= texture2D(texUnit7, vShadowUV).rgb;",
            "ApplyFog(FragmentColor);",
            "gl_FragColor = FragmentColor;",
            "}",
        ];
        assert_eq!(code(&patch_fslit(FSLIT).unwrap()), fs);
        assert!(patch_fslit(FSLIT).unwrap().contains("\n    ApplyFog(FragmentColor);\n"), "the indent is kept");
    }

    #[test]
    fn a_stock_shader_of_another_shape_is_an_error() {
        let e = patch_fslit(&FSLIT.replace("#define FOG 1", "#define FOG 2")).unwrap_err().to_string();
        assert!(e.contains("fslit.shd has 0 lines `#define FOG 1`"), "{e}");
        let twice = VSLIT.replace("ApplyStandardShader();", "ApplyStandardShader();\n    ApplyStandardShader();");
        assert!(patch_vslit(&twice).unwrap_err().to_string().contains("has 2 lines"));
    }
}
