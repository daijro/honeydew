Honeydew 0.5.1 fixes the piano roll's key names, solo through a bus and Pitcher 2's formants, and adds song sections, collapsed tracks, a Half Time effect, tempo matching for audio clips, and a chord and melody generator.

- Piano roll. With Show key names on, the piano roll's keys show their names, picking a scale dims the keys outside it, and ghost notes are lighter (#77, #94).
- Solo. Soloing a channel plays only that channel and the path its sound takes to the master, even when every channel goes through a bus (#82).
- Song sections. Select a range on the ruler and choose Section... to name and colour a part of the song. The rows in it are tinted and its name shows on the ruler (#80).
- Collapse tracks. The track menu can collapse a track to a thin row with its name (#83).
- Audio clips. More... has Match tempo..., which finds a sample's tempo and stretches it to the song's, and Chop..., which cuts a clip every 1/6 to 1 beat or at its hits (#91).
- Half Time, a new effect that plays the audio at half speed, locked to the beat (#91).
- Pitcher 2's formant shift works, both Pitchers use far less CPU, and they no longer track voices an octave low or click where words start and stop (#94).
- Recording. A channel with monitoring on records the notes and controllers its MIDI sends bring it from other tracks (#74).
- The Lemon plugins' file pickers play what you tap and load your pick when you close them (#93).
- Drums. Kits play a preview in the kit picker, the drum editor's level meters no longer cover its rows, and the pads panel rises up to the ruler (#85, #88, #92).
- DW Sampler. New from samples reads a file named C4 as middle C, and SFZ pitch_keytrack works. The SoundFont Player has a Fine Tune slider (#90).
- Import takes samples and SoundFonts up to 2.5 GB, and Type in value in the Lemon plugins works on Android (#91).
- Bluetooth. Meters, plugin displays and the playhead follow the sound you hear and move smoothly, and Settings has a movable performance overlay (#95, #89).
- The Lemon Studio plugins have their own section at the bottom of Add Module (#86).
- The PROJECT tab has a margin in landscape, folder names are a bit bigger, warning popups are centred, and Help > What's new shows these notes (#78, #79, #81).
- Piano roll. GENERATE in the transport bar writes a chord progression and a melody into the clip, with presets including Random, and a preview before you apply it.
- Randomizer. New Time and Shift knobs place notes early or late.
- Analyzer. The Spectrum's tilt is a setting (White, Pink, Default, Brown, or Custom in dB per octave).
- Play loop. A playhead outside the loop jumps back into it when you press Play.
- Android. Turning the phone no longer freezes the app or leaves the screen at the old size, and the Screen orientation setting applies reliably.
- The editors' close ribbon, the rack tab and the graph tab can be dragged out of the way, and stay where you put them.
- On phones, a long press on a Lemon plugin's knob opens its menu.
- PROJECT. Samplerate picks the project's sample rate or follows the output, and you're told if the device can't run at it.

Thanks to @jikk12kk-bot-bot (with work from RememberedA's fork) for PR #87, where Generate, the Randomizer's timing, the Analyzer tilt, the rotation fixes, the movable tabs, the Lemon long press, the play loop jump and the project sample rate come from.
