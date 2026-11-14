------------------------------- MODULE KernLock -------------------------------
(* The lock of the kernel the host's processes share (host/vnkern.js vnhost_kern_lock and
   vnhost_kern_unlock, host/pre.js exited): a futex mutex on one Int32 word that the threads of
   every process take, where a process may end at any point of any of its threads (an exit, a
   kill), and the host then frees what it held.

   New = FALSE is the lock as it was: Drepper's mutex (0 free, 1 taken, 2 taken with waiters),
   its holder's process in a second word, stored after taking it and cleared before letting go,
   and an unlock that wakes one waiter. Two ways for a waiter to sleep for good with no one to
   wake it (the kernel stood still for every process until some other process happened to take
   the lock with contention: kernel32:debugger's debug_children, a child whose WaitForSingleObject
   on its ended child slept on the lock for 20 s and more):
   - an unlock wakes a thread of a process that then ends: the one wakeup is gone;
   - a process ends between taking the lock and storing itself as its holder (or between
     clearing that and letting go): no one ever frees the lock.
   New = TRUE is the lock now: the word holds the holder's process and a waiters bit
   (pid << 1 | 1), taken with one compare-exchange; a wait gives up after 100 ms and frees the
   lock of a process that ended holding it (vnkernFreeDead); the host frees it too, by that word;
   an unlock lets go only while the word is still its own process's (a thread of an ended process
   may run on after the host freed its lock) and wakes one waiter.

   Threads loop for ever: want the lock, hold it, let it go. Any thread of a mortal process may
   find its process ended at any step; Exited is the host's part after that. *)
EXTENDS Naturals, FiniteSets

CONSTANTS Procs,    \* the processes
          Mortal,   \* the ones that may end
          Ids,      \* thread numbers in each process
          New,      \* the lock now (TRUE) or as it was (FALSE)
          Timeouts  \* (New) the waits give up after a while

Threads == Procs \X Ids
None == "none"

VARIABLES pc,       \* each thread's step ("dead" once its process ended)
          c,        \* a thread's local value (old lock: what CAS/exchange gave)
          word,     \* old: 0, 1, 2; new: [own |-> process or None, waiters |-> BOOLEAN]
          holder,   \* old: the second word (a process or None)
          sleeping, \* the threads asleep in Atomics.wait on the word
          ended,    \* the processes that ended (their exit began)
          freed,    \* the ended processes the host has dealt with (Exited: on its ring)
          stopped   \* the ended processes whose threads stopped

vars == <<pc, c, word, holder, sleeping, ended, freed, stopped>>

Free == IF New THEN [own |-> None, waiters |-> FALSE] ELSE 0

Init == /\ pc = [t \in Threads |-> "idle"]
        /\ c = [t \in Threads |-> 0]
        /\ word = Free
        /\ holder = None
        /\ sleeping = {}
        /\ ended = {}
        /\ freed = {}
        /\ stopped = {}

\* (a thread runs until its process's threads stop, which may be after its exit freed the lock)
Live(t) == t[1] \notin stopped
Go(t, s) == pc' = [pc EXCEPT ![t] = s]
Keep == UNCHANGED <<holder, ended, freed, stopped>>

\* (Atomics.notify: one of the sleepers (n = 1) or all of them (n = 0) wake; the threads of
\* `self` (the notifying one, or none) go on to step `next`)
Wake(n, self, next) ==
  \E w \in SUBSET sleeping :
    /\ Cardinality(w) = IF n = 0 THEN Cardinality(sleeping) ELSE IF sleeping = {} THEN 0 ELSE 1
    /\ sleeping' = sleeping \ w
    /\ pc' = [t \in Threads |-> IF t \in w THEN (IF New THEN "n2" ELSE "o4")
                                ELSE IF t \in self THEN next ELSE pc[t]]

-----------------------------------------------------------------------------
\* the lock as it was
O(t) ==
  \/ /\ pc[t] = "idle" /\ Go(t, "o1") /\ UNCHANGED <<c, word, sleeping>> /\ Keep
  \/ /\ pc[t] = "o1"   \* c = CAS(h, 0, 0, 1)
     /\ c' = [c EXCEPT ![t] = word]
     /\ word' = IF word = 0 THEN 1 ELSE word
     /\ Go(t, IF word = 0 THEN "o5" ELSE "o2") /\ UNCHANGED sleeping /\ Keep
  \/ /\ pc[t] = "o2"   \* if (c !== 2) c = exchange(h, 0, 2)
     /\ IF c[t] # 2 THEN c' = [c EXCEPT ![t] = word] /\ word' = 2 ELSE UNCHANGED <<c, word>>
     /\ Go(t, "o3") /\ UNCHANGED sleeping /\ Keep
  \/ /\ pc[t] = "o3"   \* while (c !== 0) Atomics.wait(h, 0, 2)
     /\ IF c[t] = 0 THEN Go(t, "o5") /\ UNCHANGED sleeping
        ELSE IF word = 2 THEN sleeping' = sleeping \cup {t} /\ Go(t, "sleep")
        ELSE Go(t, "o4") /\ UNCHANGED sleeping
     /\ UNCHANGED <<c, word>> /\ Keep
  \/ /\ pc[t] = "o4"   \* c = exchange(h, 0, 2)
     /\ c' = [c EXCEPT ![t] = word] /\ word' = 2
     /\ Go(t, "o3") /\ UNCHANGED sleeping /\ Keep
  \/ /\ pc[t] = "o5"   \* store(h, 5, pid)
     /\ holder' = t[1] /\ Go(t, "cs") /\ UNCHANGED <<c, word, sleeping, ended, freed, stopped>>
  \/ /\ pc[t] = "cs" /\ Go(t, "u1") /\ UNCHANGED <<c, word, sleeping>> /\ Keep
  \/ /\ pc[t] = "u1"   \* store(h, 5, 0)
     /\ holder' = None /\ Go(t, "u2") /\ UNCHANGED <<c, word, sleeping, ended, freed, stopped>>
  \/ /\ pc[t] = "u2"   \* if (sub(h, 0, 1) !== 1)
     /\ word' = word - 1 /\ Go(t, IF word = 1 THEN "idle" ELSE "u3")
     /\ UNCHANGED <<c, sleeping>> /\ Keep
  \/ /\ pc[t] = "u3"   \* store(h, 0, 0); notify(h, 0, 1)
     /\ word' = 0 /\ Wake(1, {t}, "idle")
     /\ UNCHANGED c /\ Keep

\* the lock now
N(t) ==
  LET me == [own |-> t[1], waiters |-> FALSE] IN
  \/ /\ pc[t] = "idle" /\ Go(t, "n1") /\ UNCHANGED <<c, word, sleeping>> /\ Keep
  \/ /\ pc[t] = "n1"   \* if (CAS(h, 0, 0, me) === 0) return
     /\ IF word.own = None THEN word' = me /\ Go(t, "cs") ELSE UNCHANGED word /\ Go(t, "n2")
     /\ UNCHANGED <<c, sleeping>> /\ Keep
  \/ /\ pc[t] = "n2"   \* v = load; free: CAS(0, me | 1); else set the waiters bit, wait on v
     /\ IF word.own = None
          THEN word' = [me EXCEPT !.waiters = TRUE] /\ Go(t, "cs") /\ UNCHANGED sleeping
          ELSE word' = [word EXCEPT !.waiters = TRUE] /\ sleeping' = sleeping \cup {t} /\ Go(t, "sleep")
     /\ UNCHANGED c /\ Keep
  \/ /\ pc[t] = "cs" /\ Go(t, "n3") /\ UNCHANGED <<c, word, sleeping>> /\ Keep
  \/ /\ pc[t] = "n3"   \* if still ours: CAS(c, 0); if (c & 1) notify(h, 0, 1)
     /\ IF word.own # t[1] THEN UNCHANGED <<word, sleeping>> /\ Go(t, "idle")
        ELSE word' = Free /\ IF word.waiters THEN Wake(1, {t}, "idle") ELSE Go(t, "idle") /\ UNCHANGED sleeping
     /\ UNCHANGED c /\ Keep

\* (the lock now) Atomics.wait(h, 0, c, 100) timed out: vnkernFreeDead, then look again
Timeout(t) ==
  /\ New /\ Timeouts /\ Live(t) /\ t \in sleeping
  /\ IF word.own # None /\ word.own \in freed
       THEN word' = Free /\ Wake(0, {}, "idle")
       ELSE UNCHANGED word /\ sleeping' = sleeping \ {t} /\ Go(t, "n2")
  /\ UNCHANGED c /\ Keep

Step(t) == /\ Live(t) /\ pc[t] # "sleep"
           /\ IF New THEN N(t) ELSE O(t)

\* a process's exit begins (its other threads may still run a little)
Die(p) == /\ p \in Mortal /\ p \notin ended
          /\ ended' = ended \cup {p}
          /\ UNCHANGED <<pc, c, word, holder, sleeping, freed, stopped>>

\* its threads stop wherever they are (Atomics.wait never returns there)
Stop(p) == /\ p \in ended /\ p \notin stopped
           /\ stopped' = stopped \cup {p}
           /\ pc' = [t \in Threads |-> IF t[1] = p THEN "dead" ELSE pc[t]]
           /\ sleeping' = {t \in sleeping : t[1] # p}
           /\ UNCHANGED <<c, word, holder, ended, freed>>

\* host/pre.js exited(): the lock of a process that ended holding it goes, and its waiters wake
Exited(p) ==
  /\ p \in ended /\ p \notin freed
  /\ freed' = freed \cup {p}
  /\ IF New
       THEN IF word.own = p THEN word' = Free /\ Wake(0, {}, "idle") /\ UNCHANGED holder
            ELSE UNCHANGED <<word, holder, pc, sleeping>>
       ELSE IF holder = p THEN holder' = None /\ word' = 0 /\ Wake(0, {}, "idle")
            ELSE UNCHANGED <<word, holder, pc, sleeping>>
  /\ UNCHANGED <<c, ended, stopped>>

Next == \/ \E t \in Threads : Step(t) \/ Timeout(t)
        \/ \E p \in Procs : Die(p) \/ Stop(p) \/ Exited(p)

Spec == Init /\ [][Next]_vars /\ \A t \in Threads : WF_vars(Step(t)) /\ WF_vars(Timeout(t))
             /\ \A p \in Procs : WF_vars(Exited(p)) /\ WF_vars(Stop(p))

-----------------------------------------------------------------------------
\* one holder at a time among the threads of processes that have not ended (a thread of an
\* ending process that took the lock after its exit freed it may find it taken from it: its
\* process is gone a moment later)
Mutex == \A s, t \in Threads : (s[1] \notin ended /\ t[1] \notin ended /\ pc[s] = "cs" /\ pc[t] = "cs") => s = t

\* a thread asleep on the lock is woken (or its process ends); the lock is not fair, a thread
\* may lose it again and again, but no one sleeps on it for ever
Wakes == \A t \in Threads : (t \in sleeping) ~> (t \notin sleeping)
\* while living threads want the lock, one gets it
Progress == (\E t \in Threads : Live(t) /\ pc[t] \notin {"idle", "cs", "dead"}) ~> (\E t \in Threads : Live(t) /\ pc[t] = "cs")
=============================================================================
