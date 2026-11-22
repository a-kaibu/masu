---------------------------- MODULE SaveApiTrace ----------------------------
(* A trace of the real save API (written by crates/masu/src/storage.rs gc_tests::saves_trace
   into SaveData.tla, with the bucket's fake failing requests and refusing forgets at random)
   against SaveApi: each call's answer, the row's revision and the revisions named after it. *)
EXTENDS SaveApi, SaveData, Sequences

VARIABLES i, s

\* an outcome that is what the trace saw after call t
Fits(o, t) ==
  /\ o.ret = t.ret /\ o.rev = t.rev /\ o.got = t.got
  /\ o.st.row = t.row /\ o.lo \subseteq t.kept /\ t.kept \subseteq o.hi

Init == i = 0 /\ s = [n \in Names |-> Empty]
Step ==
  /\ i < Len(Trace)
  /\ LET t == Trace[i + 1] IN
       \E o \in Outcomes(s[t.name], t.op, t.e, t.v) :
         Fits(o, t) /\ s' = [s EXCEPT ![t.name] = [o.st EXCEPT !.kept = t.kept]]
  /\ i' = i + 1
End == i = Len(Trace) /\ UNCHANGED <<i, s>>
Next == Step \/ End

\* the next call has an outcome SaveApi allows that fits what was seen
Follows == i < Len(Trace) =>
  LET t == Trace[i + 1] IN \E o \in Outcomes(s[t.name], t.op, t.e, t.v) : Fits(o, t)
Named == \A n \in Names : PublishedNamed(s[n])
=============================================================================
