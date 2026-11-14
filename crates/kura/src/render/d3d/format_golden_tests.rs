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
            "{i} {} bpp={} storable={} renderable={} ms={} resolvable={} blendable={} depth={} halved={}\n",
            f.name(),
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

/// The view dimensions and topologies the command words' codes name, pinned before they become
/// enums: each code's WebGPU name (and one past them: the default), whether a topology is a strip.
#[test]
fn view_dimensions_and_topologies_as_pinned() {
    let dims: Vec<String> = (0..8).map(|i| pick(&VIEW_DIM, i, "2d").to_string()).collect();
    assert_eq!(dims.join(" "), "1d 2d 2d-array cube cube-array 3d 2d 2d");
    let topo: Vec<String> = (0..7).map(|i| { let t = pick(&TOPOLOGY, i, "triangle-list"); format!("{t}:{}", u8::from(t.ends_with("strip"))) }).collect();
    assert_eq!(topo.join(" "), "point-list:0 line-list:0 line-strip:1 triangle-list:0 triangle-strip:1 triangle-list:0 triangle-list:0");
}
