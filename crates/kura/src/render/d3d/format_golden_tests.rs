//! Every answer render::d3d gives about a WebGPU texture format pinned, before the formats become
//! an enum instead of strings (`Tex::format`, TEX_FORMATS, and the suffix tests on them): for each
//! page format code (and one past them), its WebGPU format and bpp, storable, renderable,
//! multisamplable, resolvable, blendable, is_depth, halved. (Native builds have no GPU features:
//! the feature-dependent answers are those without them.) format_golden.txt holds them;
//! MASU_UPDATE_GOLDEN=1 rewrites it.
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
    kome::golden::check(&path, &got);
}

/// The view dimensions and topologies the command words' codes name, pinned before they become
/// enums: each code's WebGPU name (and one past them: the default), whether a topology is a strip.
#[test]
fn view_dimensions_and_topologies_as_pinned() {
    let dims: Vec<String> = (0..8).map(|i| ViewDim::of(i).name().to_string()).collect();
    assert_eq!(dims.join(" "), "1d 2d 2d-array cube cube-array 3d 2d 2d");
    let topo: Vec<String> = (0..7).map(|i| { let t = Topology::of(i); format!("{}:{}", t.name(), u8::from(t.is_strip())) }).collect();
    assert_eq!(topo.join(" "), "point-list:0 line-list:0 line-strip:1 triangle-list:0 triangle-strip:1 triangle-list:0 triangle-list:0");
}

/// The key's blend factors, blend operations, stencil operations and comparisons (codes 0..=20
/// and two far past them): each one's WebGPU name, with each default the decoders pass.
#[test]
fn key_states_as_pinned() {
    let codes: Vec<u32> = (0..=20).chain([0x8006, u32::MAX]).collect();
    let names = |f: &dyn Fn(u32) -> &'static str| codes.iter().map(|&c| f(c)).collect::<Vec<_>>().join(" ");
    let got = [
        names(&|c| factor_name(c, "one")),
        names(&|c| factor_name(c, "zero")),
        names(&blend_op_name),
        names(&stencil_op_name),
        names(&|c| compare_name(c, "always")),
        names(&|c| compare_name(c, "less")),
    ];
    let want = [
        "zero one src one-minus-src src-alpha one-minus-src-alpha dst-alpha one-minus-dst-alpha dst one-minus-dst src-alpha-saturated constant one-minus-constant one one one one one one one one one one",
        "zero one src one-minus-src src-alpha one-minus-src-alpha dst-alpha one-minus-dst-alpha dst one-minus-dst src-alpha-saturated constant one-minus-constant one zero zero zero zero zero zero zero zero zero",
        "add add subtract reverse-subtract min max add add add add add add add add add add add add add add add add add",
        "keep keep zero replace increment-clamp decrement-clamp invert increment-wrap decrement-wrap keep keep keep keep keep keep keep keep keep keep keep keep keep keep",
        "always never less equal less-equal greater not-equal greater-equal always always always always always always always always always always always always always always always",
        "always never less equal less-equal greater not-equal greater-equal always less less less less less less less less less less less less less less",
    ];
    for (g, w) in got.iter().zip(want) {
        assert_eq!(g, w);
    }
}
