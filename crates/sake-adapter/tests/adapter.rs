//! The adapter end to end, on the real host (the game's directory, the real clock): the new
//! sake's C guests (sake-testkit) give the console Windows gives (their goldens), the GUI guest's
//! frames reach masu, and a stopped game ends as cancelled.

#[cfg(test)]
mod adapter {
    use sake::end::End;
    use sake::host::{InputEvent, Stream};
    use sake::run::Speakers;
    use sake::session::Refused;
    use sake_adapter::{Game, Out, Over, Play, Turn, Wake};
    use sake_testkit::{GUI, GUI_EXIT, GUI_GOLDEN, HEADLESS, HEADLESS_DATA, HEADLESS_EXIT, HEADLESS_GOLDEN};

    /// Plays to the end, sleeping when the game waits; everything it gave out, in turn order.
    fn play(game: Game) -> (Over, Vec<Out>, u32) {
        let mut out = Vec::new();
        let mut turns = 0u32;
        let mut turn = Play::start(game);
        loop {
            turn = match turn {
                Turn::Over(mut over) => {
                    out.append(&mut over.out);
                    return (over, out, turns);
                }
                Turn::Playing(play, mut given, wake) => {
                    out.append(&mut given);
                    turns = turns.saturating_add(1);
                    assert_ne!(wake, Wake::Input, "the scenario waits for input nobody gives");
                    if let Wake::After(wait) = wake {
                        std::thread::sleep(wait);
                    }
                    play.turn(20_000)
                }
            };
        }
    }

    fn playing(turn: Turn) -> Option<Play> {
        if let Turn::Playing(play, _, _) = turn { Some(play) } else { None }
    }

    fn stdout(out: &[Out]) -> String {
        let bytes: Vec<u8> = out.iter().filter_map(|out| if let Out::Console { stream: Stream::Out, bytes } = out { Some(bytes.clone()) } else { None }).flatten().collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn the_headless_guest_plays_as_on_windows() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("data.txt"), HEADLESS_DATA).unwrap();
        let (over, out, turns) = play(Game { root: root.path().to_path_buf(), exe: HEADLESS.to_vec(), speakers: Speakers::Virtual });
        assert_eq!(over.end, End::Exited { code: HEADLESS_EXIT, exception: None });
        assert_eq!(stdout(&out), String::from_utf8_lossy(HEADLESS_GOLDEN));
        assert!(turns > 1, "{turns}");
    }

    #[test]
    fn the_gui_guest_shows_its_frames_to_masu() {
        let root = tempfile::tempdir().unwrap();
        let (over, out, _) = play(Game { root: root.path().to_path_buf(), exe: GUI.to_vec(), speakers: Speakers::None });
        assert_eq!(over.end, End::Exited { code: GUI_EXIT, exception: None });
        assert_eq!(stdout(&out), String::from_utf8_lossy(GUI_GOLDEN));
        let frames: Vec<_> = out.iter().filter_map(|out| if let Out::Frame(frame) = out { Some(frame) } else { None }).collect();
        let last = frames.last().unwrap();
        assert_eq!((last.left, last.top, last.width, last.height), (100, 100, 160, 120));
        assert_eq!(last.pixels.len(), 160 * 120 * 4);
        assert!(frames.windows(2).all(|pair| matches!(pair, [first, second] if first.at <= second.at)));
    }

    #[test]
    fn a_stopped_game_ends_cancelled() {
        let root = tempfile::tempdir().unwrap();
        let play = playing(Play::start(Game { root: root.path().to_path_buf(), exe: GUI.to_vec(), speakers: Speakers::None })).unwrap();
        let mut play = playing(play.turn(1)).unwrap();
        let steps = play.steps();
        assert!(steps >= 1);
        // (the player's input is queued until the game takes it, up to the session's flow)
        let mut queued = 0usize;
        while play.press(InputEvent::Move { x: 150, y: 150 }) == Ok(()) {
            queued = queued.saturating_add(1);
        }
        assert_eq!(play.press(InputEvent::Move { x: 150, y: 150 }), Err(Refused::Full));
        assert_eq!(queued, 256);
        let over = play.stop();
        assert_eq!((over.end, over.steps), (End::Cancelled, steps));
    }

    #[test]
    fn a_broken_exe_is_over_at_start() {
        let root = tempfile::tempdir().unwrap();
        let over = match Play::start(Game { root: root.path().to_path_buf(), exe: b"MZ".to_vec(), speakers: Speakers::None }) {
            Turn::Over(over) => Some(over),
            Turn::Playing(..) => None,
        }
        .unwrap();
        assert!(matches!(over.end, End::BadImage(_)), "{:?}", over.end);
        assert_eq!(over.steps, 0);
    }
}
