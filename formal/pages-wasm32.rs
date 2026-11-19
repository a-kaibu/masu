#[allow(dead_code)]
#[path = "../crates/kura/src/pages.rs"]
mod pages;

fn main() {
    assert_eq!(usize::BITS, 32);
    let mut file = pages::Pages::from_slice(b"abcd");
    for len in [1_u64 << 32, (1_u64 << 32) + 1] {
        file.set_len(len).unwrap();
        let mut out = [0; 4];
        assert_eq!(file.read(0, &mut out), 4);
        assert_eq!(&out, b"abcd");
        assert_eq!(file.read(len - 1, &mut out), 1);
        assert_eq!(out[0], 0);
    }
}
