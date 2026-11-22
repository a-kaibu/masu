------------------------------ MODULE SaveApi ------------------------------
(* What masu's save API (crates/masu/src/saves.rs, Saves with a bucket) may answer, one call
   at a time, per save name: its state is the revision the row holds (row), the revisions the
   bucket still names (kept: "saves/<name>/<rev>" in the objects table) and what each named
   revision holds (val). SaveCommit is the protocol under concurrency and failures; this is the
   contract a trace of the real code (storage.rs gc_tests::saves_trace) is checked against
   through SaveApiTrace.

   write(e, v)  the revision expected is the row's: "yes", the row moves to e + 1 holding v and
                the bucket names e + 1; the revisions at or below e + 1 - K are forgotten, all,
                some or none of them (a failed forget is logged and is no failed write).
                "err": nothing a reader sees moved (row and val), though the new revision may
                be named (its record landed and the row did not).
                Another revision expected: "no", nothing moves.
   read         no row: "none"; else "got" the row's revision and its value, or "err" (the
                bucket's GET failed).
   gc           "ok" or "err"; nothing a reader sees moves (no named revision is collected).
   Bounds: none of its own; a trace fixes the calls. Assumed: calls one at a time (SaveCommit
   has them at once). *)
EXTENDS Naturals, FiniteSets

CONSTANT K

Empty == [row |-> 0, kept |-> {}, val |-> [r \in {} |-> 0]]

Set(f, r, v) == [x \in DOMAIN f \cup {r} |-> IF x = r THEN v ELSE f[x]]

\* the answers a call may get and the states it may leave, as [ret, rev, got, st, lo, hi]: the
\* bucket names at least lo and at most hi afterwards (st.kept is lo: where the two differ,
\* forgets that failed, the trace's kept is taken)
Out(ret, rev, got, st, lo, hi) == [ret |-> ret, rev |-> rev, got |-> got, st |-> st, lo |-> lo, hi |-> hi]
Same(ret, rev, got, s) == Out(ret, rev, got, s, s.kept, s.kept)

Outcomes(s, op, e, v) ==
  CASE op = "write" /\ s.row = e ->
         LET n == e + 1
             all == s.kept \cup {n}
             young == {r \in all : r > n - K}
         IN {Out("yes", 0, 0, [row |-> n, kept |-> young, val |-> Set(s.val, n, v)], young, all),
             Out("err", 0, 0, s, s.kept, all)}
    [] op = "write" -> {Same("no", 0, 0, s)}
    [] op = "read" /\ s.row = 0 -> {Same("none", 0, 0, s)}
    [] op = "read" -> {Same("got", s.row, s.val[s.row], s), Same("err", 0, 0, s)}
    [] op = "gc" -> {Same("ok", 0, 0, s), Same("err", 0, 0, s)}

\* what the implementation keeps true between calls: the row's revision is named
PublishedNamed(s) == s.row > 0 => s.row \in s.kept
=============================================================================
