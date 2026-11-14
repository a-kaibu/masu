//! Every answer render::d3d gives about a WebGPU texture format pinned, before the formats become
//! an enum instead of strings (`Tex::format`, TEX_FORMATS, and the suffix tests on them): for each
//! page format code (and one past them), its WebGPU format and bpp, storable, renderable,
//! multisamplable, resolvable, blendable, is_depth, halved. (Native builds have no GPU features:
//! the feature-dependent answers are those without them.) format_golden.txt holds them;
//! MASU_UPDATE_FORMATS=1 rewrites it.
use super::*;

#[test]
fn every_format_answers_as_pinned() {
    let mut got = String::new();
    for i in 0..=TEX_FORMATS.len() as u32 {
        let f = tex_format(i);
        got += &format!(
            "{i} {f} bpp={} storable={} renderable={} ms={} resolvable={} blendable={} depth={} halved={}\n",
            bpp(f),
            u8::from(storable(f)),
            u8::from(renderable(f)),
            u8::from(multisamplable(f)),
            u8::from(resolvable(f)),
            u8::from(blendable(f)),
            u8::from(is_depth(f)),
            u8::from(halved(i)),
        );
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/render/d3d/format_golden.txt");
    if std::env::var_os("MASU_UPDATE_FORMATS").is_some() {
        std::fs::write(&path, &got).unwrap();
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(want == got, "render::d3d format answers differ from format_golden.txt (MASU_UPDATE_FORMATS=1 rewrites it):\n{got}");
}
