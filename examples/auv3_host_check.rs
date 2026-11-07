//! Optional integration check using an installed AUv3 synth, without an audio device.
//! Run on macOS: cargo run --example auv3_host_check -- 'Super J8'
#[cfg(target_os = "macos")]
fn main() {
    use honeydew::audio_engine::SongAudioEngine;
    use honeydew::plugins::{self, PluginKind, State};
    use honeydew::song::{ChannelKind, Event, Song};
    use honeydew_audio::device::AudioRenderer;
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSDate, NSRunLoop};
    use std::time::{Duration, Instant};

    let marker = MainThreadMarker::new().expect("run on the main thread");
    let app = NSApplication::sharedApplication(marker);
    app.finishLaunching();
    let tick =
        || NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
    let name = std::env::args().nth(1).unwrap_or_else(|| "Super J8".into());
    plugins::rescan_audio_units();
    // An asynchronous desktop file scan must not erase the system components.
    plugins::load_cache(&plugins::cache_json());
    let available = plugins::available()
        .into_iter()
        .find(|p| {
            p.format == "AUv3"
                && p.kind == PluginKind::Instrument
                && p.name.to_lowercase().contains(&name.to_lowercase())
        })
        .expect("install and open the synth's containing app first");
    println!("Discovered {} ({})", available.name, available.uid);
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let channel = &mut song.channels[1];
    let channel_id = channel.id;
    let instance = plugins::add(&mut channel.rack, channel_id, &available.uid, 0).unwrap();
    channel.lanes[0].add_clip(0, 4.0).events.push(Event::Note {
        pos: 0,
        len_beats: 1.0,
        key: 60,
        velocity: 204,
        pan: 127,
        slide: false,
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        tick();
        plugins::idle();
        plugins::adopt_params(&mut song);
        match plugins::state(channel_id, instance) {
            Some(State::Live(info)) if info.stopped.is_some() => {
                panic!("AUv3 stopped: {:?}", info.stopped)
            }
            Some(State::Missing(why)) => panic!("AUv3 missing: {why}"),
            _ => {}
        }
        if matches!(plugins::state(channel_id, instance), Some(State::Live(info)) if info.has_editor)
        {
            break;
        }
        assert!(Instant::now() < deadline, "AUv3 load timed out");
    }
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.play();
    let mut energy = 0.0;
    for _ in 0..200 {
        let mut output = [0.0; 512];
        engine.render(&mut output, 2);
        assert!(output.iter().all(|s| s.is_finite()));
        energy += output.iter().map(|s| s * s).sum::<f32>();
    }
    assert!(energy > 0.00001, "the rack produced no audible samples");
    drop(engine);
    plugins::store_states(&mut song);
    let state = &song.channels[1].rack.modules[0]
        .file
        .plugin
        .as_ref()
        .unwrap()
        .state;
    assert!(!state.is_empty());
    let back = honeydew::flm::read(&honeydew::flm::write(&song).unwrap()).unwrap();
    assert_eq!(
        back.channels[1].rack.modules[0]
            .file
            .plugin
            .as_ref()
            .unwrap()
            .state,
        *state
    );
    plugins::show_editor(channel_id, instance, true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        tick();
        plugins::idle();
    }
    assert!(
        matches!(plugins::state(channel_id, instance), Some(State::Live(info)) if info.editor_open)
    );
    plugins::show_editor(channel_id, instance, false).unwrap();
    plugins::song_replaced();
    plugins::idle();
    let mut back = back;
    plugins::sync(&back);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        tick();
        plugins::idle();
        plugins::adopt_params(&mut back);
        if matches!(plugins::state(channel_id, instance), Some(State::Live(info)) if info.has_editor)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "saved AUv3 instance did not revive"
        );
    }
    let mut restored = SongAudioEngine::new(&back, 48_000.0).unwrap();
    restored.play();
    let mut restored_energy = 0.0;
    for _ in 0..200 {
        let mut output = [0.0; 512];
        restored.render(&mut output, 2);
        assert!(output.iter().all(|s| s.is_finite()));
        restored_energy += output.iter().map(|s| s * s).sum::<f32>();
    }
    assert!(
        restored_energy > 0.00001,
        "restored AUv3 project produced no audio"
    );
    drop(restored);
    plugins::song_replaced();
    plugins::idle();
    println!(
        "AUv3 Honeydew integration passed: audio energy {energy}, restored energy {restored_energy}, project reopened, editor opened/closed"
    );
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This optional installed-plugin check runs on macOS.");
}
