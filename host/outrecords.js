// The page's threads' output records (crates/kura/src/wrap.rs: fd, u16 length, bytes; host/pre.js
// drains them) as bytes for emscripten's printChar, which ends a console line at '\n' and also
// at a 0 byte: a NUL in what a program wrote (a wide string's terminator printed with it) cut its
// line in two on the page (tests/exec-browser.mjs saw "chain 2: args [/windo" and "w] ..."; review
// e18bf44). NULs are left out, every other byte goes as it is; a record cut short at the end of
// what was taken stops the walk (none is: Ring::take moves whole records).
function vnhostOutputRecords(b, n, put) {
  for (let at = 0; at + 3 <= n; ) {
    const fd = b[at], len = b[at + 1] | (b[at + 2] << 8);
    if (at + 3 + len > n) return;
    for (let i = at + 3; i < at + 3 + len; i++) if (b[i] !== 0) put(fd, b[i]);
    at += 3 + len;
  }
}
