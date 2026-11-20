//! A guest byte stream is released once, by whichever holder of its Data lets go last, however
//! they let go (release_data, a plain drop, many threads at once): the Arc audit of 2026-11-20
//! found release_data's Arc::try_unwrap could lose it (two last holders both got Err) and plain
//! drops (a session's Plan, a reader's streams) never released it.
use super::*;
use proptest::prelude::*;
use std::sync::Barrier;

/// a Data on a made-up guest stream `bs` whose release goes to `to` (not the process's LET_GO,
/// which the other tests' readers drain)
fn guest(bs: u32, to: &'static LetGo) -> Data {
    Data { src: Src::Guest(bs, to), len: 0, lock: Mutex::new(()) }
}

fn sorted(mut v: Vec<u32>) -> Vec<u32> {
    v.sort_unstable();
    v
}

#[test]
fn the_last_holder_lets_go_once() {
    static TO: LetGo = LetGo::new();
    let d = Arc::new(guest(7, &TO));
    let mut held: Vec<Arc<Data>> = (0..5).map(|_| d.clone()).collect();
    drop(d);
    let last = held.pop();
    drop(held);
    assert_eq!(TO.take(), Vec::<u32>::new(), "nobody let go last yet");
    drop(last);
    assert_eq!(TO.take(), vec![7]);
}

#[test]
fn a_plain_drop_lets_go_too() {
    static TO: LetGo = LetGo::new();
    // (a session's Plan: a clone dropped without release_data, after the source's own went)
    let source = Arc::new(guest(9, &TO));
    let plan = source.clone();
    drop(source);
    assert!(TO.take().is_empty());
    drop(plan);
    assert_eq!(TO.take(), vec![9]);
    assert!(TO.take().is_empty(), "taken once");
}

#[test]
fn bytes_of_ours_and_files_let_nothing_go() {
    static TO: LetGo = LetGo::new();
    drop(Data::mem_arc(Arc::new(vec![1, 2, 3])));
    drop(guest(0, &TO));
    assert!(TO.take().is_empty(), "no stream (0) is released as none");
}

#[test]
fn last_holders_on_many_threads_at_once_let_go_once() {
    // (the race try_unwrap lost: every holder drops at the same moment)
    static TO: LetGo = LetGo::new();
    const THREADS: usize = 8;
    for round in 1..=200u32 {
        let d = Arc::new(guest(round, &TO));
        let gate = Barrier::new(THREADS);
        std::thread::scope(|s| {
            for _ in 0..THREADS {
                let mine = d.clone();
                let gate = &gate;
                s.spawn(move || {
                    gate.wait();
                    drop(mine);
                });
            }
            drop(d);
        });
        assert_eq!(TO.take(), vec![round], "round {round}");
    }
}

#[derive(Clone, Debug)]
enum Step {
    /// holder i clones its Data into a new holder
    Clone(usize),
    /// holder i lets go (a plain drop)
    Drop(usize),
    /// holder i is sent to another thread and dropped there
    DropElsewhere(usize),
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![(0..16usize).prop_map(Step::Clone), (0..16usize).prop_map(Step::Drop), (0..16usize).prop_map(Step::DropElsewhere)]
}

proptest! {
    // Model: every stream is released exactly when its last holder goes, and exactly once,
    // whatever the order of clones and drops across streams and threads.
    #[test]
    fn released_exactly_when_the_last_holder_goes(streams in 1..4u32, steps in proptest::collection::vec(step(), 0..64)) {
        static TO: LetGo = LetGo::new();
        static ONE: Mutex<()> = Mutex::new(());
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        TO.take();
        let mut holders: Vec<(u32, Arc<Data>)> = (1..=streams).map(|b| (b, Arc::new(guest(b, &TO)))).collect();
        let mut released: Vec<u32> = Vec::new();
        for s in steps {
            let live: Vec<u32> = holders.iter().map(|h| h.0).collect();
            match s {
                Step::Clone(i) if !holders.is_empty() => {
                    let h = holders[i % holders.len()].clone();
                    holders.push(h);
                }
                Step::Drop(i) | Step::DropElsewhere(i) if !holders.is_empty() => {
                    let (b, d) = holders.remove(i % holders.len());
                    if matches!(s, Step::DropElsewhere(_)) {
                        std::thread::spawn(move || drop(d)).join().unwrap();
                    } else {
                        drop(d);
                    }
                    let last = !holders.iter().any(|h| h.0 == b);
                    let now = TO.take();
                    prop_assert_eq!(now.clone(), if last { vec![b] } else { vec![] }, "after {:?} with {:?}", s, live);
                    released.extend(now);
                }
                _ => {}
            }
        }
        let rest: Vec<u32> = holders.iter().map(|h| h.0).collect();
        drop(holders);
        released.extend(TO.take());
        prop_assert_eq!(sorted(released), (1..=streams).collect::<Vec<_>>(), "each once (still held at the end: {:?})", rest);
    }
}
