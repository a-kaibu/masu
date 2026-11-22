------------------------------- MODULE Binds11 -------------------------------
(* What Direct3D 11 does with a resource bound for reading and writing at once, as kura's
   context (crates/kura/src/d3d11/hazards.rs) must follow it; Binds11Trace checks a trace of
   the real context (d3d11/bind_tests.rs) against it, call by call.

   An output bind (render target, depth view unless read-only, either UAV, stream output
   target), alone or with the render targets kept (OMSetRenderTargetsAndUnorderedAccessViews'
   KEEP), unbinds the shader resource views of every stage and the vertex buffers (every slot,
   0 and 31 here) that read the same subresources; an SRV or vertex buffer bound while it reads
   a bound output is NULL instead. Views of other mips of the same texture do not conflict.

   Views (bind_tests.rs): 1 SRV of A mip 0, 2 SRV of A mip 1, 3 SRV of Z; 4 / 5 render
   targets of A mip 0 / 1; 6 / 7 depth views of Z (7 read-only); 8 UAV of A mip 0; 9 the
   buffer. Slots: 0..3 SRVs (ps0, ps1, vs0, cs0), 4 rt, 5 ds, 6 uav, 7 vb, 8 so, 9 vb31,
   10 the output merger's UAV 1 (OMSetRenderTargets unbinds it). *)
EXTENDS Naturals, Sequences

\* (what each view covers: resource, mip)
Span == <<<<1, 0>>, <<1, 1>>, <<2, 0>>, <<1, 0>>, <<1, 1>>, <<2, 0>>, <<2, 0>>, <<1, 0>>, <<3, 0>>>>
Inputs == {0, 1, 2, 3, 7, 9}

Empty == [slots |-> [s \in 0..10 |-> 0], ok |-> TRUE]

Outputs(sl) == {Span[v] : v \in {sl[4], IF sl[5] = 7 THEN 0 ELSE sl[5], sl[6], sl[8], sl[10]} \ {0}}
Reads(v, sl) == v # 0 /\ Span[v] \in Outputs(sl)
\* (after an output bind: the inputs that read an output go)
Resolve(sl) == [s \in 0..10 |-> IF s \in Inputs /\ Reads(sl[s], sl) THEN 0 ELSE sl[s]]

Do(st, t) ==
    LET sl == st.slots IN
    CASE t.op = "SetSRV" -> [st EXCEPT !.slots[t.s] = IF Reads(t.v, sl) THEN 0 ELSE t.v]
      [] t.op = "SetOM" -> [st EXCEPT !.slots = Resolve([sl EXCEPT ![4] = t.v, ![5] = t.w, ![10] = 0])]
      [] t.op = "SetUAV" -> [st EXCEPT !.slots = Resolve([sl EXCEPT ![6] = t.v])]
      [] t.op = "SetOMUAV" -> [st EXCEPT !.slots = Resolve([sl EXCEPT ![10] = t.v])]
      [] t.op = "SetVB" -> [st EXCEPT !.slots[7] = IF Reads(t.v, sl) THEN 0 ELSE t.v]
      [] t.op = "SetVB31" -> [st EXCEPT !.slots[9] = IF Reads(t.v, sl) THEN 0 ELSE t.v]
      [] t.op = "SetSO" -> [st EXCEPT !.slots = Resolve([sl EXCEPT ![8] = t.v])]
      [] t.op = "Get" -> [st EXCEPT !.ok = (t.got = sl[t.s])]
=============================================================================
