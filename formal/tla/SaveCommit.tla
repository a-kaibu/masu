---------------------------- MODULE SaveCommit ----------------------------
(* Writing a save (crates/masu/src/saves.rs Saves::write_bytes, read, and the bucket's naming in
   storage.rs: put = send + record, prune_save, gc) as its steps, each of which may fail:

   a writer takes the name's lock (NameLocks), reads the published revision and refuses an
   expected one that is not it (412); uploads the bytes as a new physical object (send: a fresh
   object id; it may fail, or fail for the client and still land later); names it as
   saves/<name>/<rev + 1> (record: the logical name's old object is retired in the same
   transaction); publishes rev + 1 in the saves row (the commit); answers yes, then forgets the
   revisions older than the kept SAVE_REVISIONS (prune_save, whose failure is no failure of the
   write). gc deletes retired objects at any time (its grace period is not modelled: a reader
   whose object went is a failed read, never a wrong one). A reader reads the row, looks the
   revision's name up and fetches the object, without the lock.

   Checked: a published revision is named by an object in the bucket that is not retired; a
   revision a writer was told is stored keeps the object that writer uploaded while it is kept
   (nothing overwrites it: a stale writer is refused, a late upload lands unnamed, a retried
   commit names the revision afresh only before it is published); no two writers are told the
   same revision (no lost update); a read gives the object of the revision it names or fails.
   SaveCommitNoLock.cfg (two processes, or a lock that is not per name, as with several server
   instances and no shared lock) must break NoLostUpdate. *)
EXTENDS Naturals, FiniteSets

CONSTANTS Writers, Readers, MaxRev, K, Lock, MaxObj

NONE == 0
Revs == 1..MaxRev
Objs == 1..MaxObj

VARIABLES
    row,        \* the saves row's revision (0: no save yet)
    named,      \* revision -> object named saves/<name>/<rev>, NONE if none
    bucket,     \* objects in the bucket
    retired,    \* objects retired (gc may delete them)
    inflight,   \* uploads that failed for their writer but may still land
    next,       \* the next object id
    holder,     \* who holds the name's lock (NONE)
    pc, exp, obj,   \* per writer: step, revision read, object uploaded
    acked,      \* revision -> object of the writer told yes, NONE
    told,       \* revision -> how many writers were told yes for it
    rpc, rrev, robj, got  \* per reader: step, revision read, object looked up, result

vars == <<row, named, bucket, retired, inflight, next, holder, pc, exp, obj, acked, told, rpc, rrev, robj, got>>

Init ==
    /\ row = 0
    /\ named = [r \in Revs |-> NONE]
    /\ bucket = {}
    /\ retired = {}
    /\ inflight = {}
    /\ next = 1
    /\ holder = NONE
    /\ pc = [w \in Writers |-> "idle"]
    /\ exp = [w \in Writers |-> 0]
    /\ obj = [w \in Writers |-> NONE]
    /\ acked = [r \in Revs |-> NONE]
    /\ told = [r \in Revs |-> 0]
    /\ rpc = [x \in Readers |-> "idle"]
    /\ rrev = [x \in Readers |-> 0]
    /\ robj = [x \in Readers |-> NONE]
    /\ got = [x \in Readers |-> "none"]

wv == <<row, named, bucket, retired, inflight, next, holder, pc, exp, obj, acked, told>>
rv == <<rpc, rrev, robj, got>>

Done(w) == /\ pc' = [pc EXCEPT ![w] = "idle"]
           /\ holder' = IF Lock THEN NONE ELSE holder

\* the lock, the row read, the client's expected revision checked (one it saw: the current or
\* the one before, a client behind)
Begin(w) ==
    /\ pc[w] = "idle"
    /\ row < MaxRev
    /\ (Lock => holder = NONE)
    /\ \E e \in {row, row - 1} \cap Nat :
        IF e = row
        THEN /\ pc' = [pc EXCEPT ![w] = "upload"]
             /\ exp' = [exp EXCEPT ![w] = row]
             /\ holder' = IF Lock THEN w ELSE holder
        ELSE /\ pc' = pc /\ exp' = exp /\ holder' = holder  \* refused (412): nothing changes
    /\ UNCHANGED <<row, named, bucket, retired, inflight, next, obj, acked, told, rv>>

\* send: a new object; it lands, or fails (and may land later, unnamed)
Upload(w) ==
    /\ pc[w] = "upload"
    /\ next <= MaxObj
    /\ next' = next + 1
    /\ obj' = [obj EXCEPT ![w] = next]
    /\ \/ /\ bucket' = bucket \cup {next}
          /\ pc' = [pc EXCEPT ![w] = "record"]
          /\ UNCHANGED <<holder, inflight>>
       \/ /\ inflight' = inflight \cup {next}
          /\ Done(w)
          /\ UNCHANGED bucket
    /\ UNCHANGED <<row, named, retired, exp, acked, told, rv>>

Land(o) ==
    /\ o \in inflight
    /\ inflight' = inflight \ {o}
    /\ bucket' = bucket \cup {o}
    /\ UNCHANGED <<row, named, retired, next, holder, pc, exp, obj, acked, told, rv>>

\* record: the object named as revision exp + 1, what that name had retired; or the
\* transaction fails (the object stays unnamed)
Record(w) ==
    /\ pc[w] = "record"
    /\ LET r == exp[w] + 1 IN
       \/ /\ named' = [named EXCEPT ![r] = obj[w]]
          /\ retired' = IF named[r] = NONE THEN retired ELSE retired \cup {named[r]}
          /\ pc' = [pc EXCEPT ![w] = "commit"]
          /\ UNCHANGED holder
       \/ /\ Done(w)
          /\ UNCHANGED <<named, retired>>
    /\ UNCHANGED <<row, bucket, inflight, next, exp, obj, acked, told, rv>>

\* the row: rev + 1 published and the writer told yes; or the commit fails (named, unpublished)
Commit(w) ==
    /\ pc[w] = "commit"
    /\ LET r == exp[w] + 1 IN
       \/ /\ row' = r
          /\ acked' = [acked EXCEPT ![r] = obj[w]]
          /\ told' = [told EXCEPT ![r] = @ + 1]
          /\ pc' = [pc EXCEPT ![w] = "prune"]
          /\ UNCHANGED holder
       \/ /\ Done(w)
          /\ UNCHANGED <<row, acked, told>>
    /\ UNCHANGED <<named, bucket, retired, inflight, next, exp, obj, rv>>

\* prune_save: the revisions up to row - K forgotten (retired); or it fails (logged)
Prune(w) ==
    /\ pc[w] = "prune"
    /\ LET cur == exp[w] + 1
           old == {r \in Revs : r + K <= cur /\ named[r] # NONE} IN
       \/ /\ named' = [r \in Revs |-> IF r \in old THEN NONE ELSE named[r]]
          /\ retired' = retired \cup {named[r] : r \in old}
       \/ UNCHANGED <<named, retired>>
    /\ Done(w)
    /\ UNCHANGED <<row, bucket, inflight, next, exp, obj, acked, told, rv>>

Gc(o) ==
    /\ o \in retired /\ o \in bucket
    /\ bucket' = bucket \ {o}
    /\ UNCHANGED <<row, named, retired, inflight, next, holder, pc, exp, obj, acked, told, rv>>

\* a reader: the row, the revision's object, then its bytes
Look(x) ==
    /\ rpc[x] = "idle" /\ row > 0
    /\ rrev' = [rrev EXCEPT ![x] = row]
    /\ robj' = [robj EXCEPT ![x] = named[row]]
    /\ rpc' = [rpc EXCEPT ![x] = "fetch"]
    /\ UNCHANGED <<got, wv>>

Fetch(x) ==
    /\ rpc[x] = "fetch"
    /\ got' = [got EXCEPT ![x] = IF robj[x] # NONE /\ robj[x] \in bucket THEN "obj" ELSE "missing"]
    /\ rpc' = [rpc EXCEPT ![x] = "done"]
    /\ UNCHANGED <<rrev, robj, wv>>

Next ==
    \/ \E w \in Writers : Begin(w) \/ Upload(w) \/ Record(w) \/ Commit(w) \/ Prune(w)
    \/ \E o \in Objs : Land(o) \/ Gc(o)
    \/ \E x \in Readers : Look(x) \/ Fetch(x)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ row \in 0..MaxRev
    /\ named \in [Revs -> 0..MaxObj]
    /\ bucket \subseteq Objs /\ retired \subseteq Objs /\ inflight \subseteq Objs

\* the published revision is named by a live object
PublishedNamed == row > 0 => (named[row] # NONE /\ named[row] \in bucket /\ named[row] \notin retired)

\* a revision a writer was told is stored keeps that writer's object while it is kept
KeptIsAcked == \A r \in Revs : (acked[r] # NONE /\ r + K > row) => named[r] = acked[r]

\* no two writers told the same revision
NoLostUpdate == \A r \in Revs : told[r] <= 1

\* a read gives the object the revision it read was acknowledged with, or fails
ReadIsAcked == \A x \in Readers : rpc[x] = "done" /\ got[x] = "obj" => robj[x] = acked[rrev[x]]

\* reachability: the model does write, prune, collect and read (not vacuously safe)
Useful == ~(row = MaxRev /\ \E x \in Readers : got[x] = "obj" /\ rrev[x] = MaxRev /\ retired # {})
=============================================================================
