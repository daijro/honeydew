Honeydew 0.5.5 adds presets for plugin modules, ghost notes from a channel's other clips, a value bubble while you drag a control, library folders and an exit question on Android, and many fixes from the issue tracker.

- Rack. Copy, Paste and rack presets keep Honeydew's own modules and plugin modules, Clear Rack keeps a plugin instrument, and plugin modules can save and load presets of their own (#149).
- Piano roll. Ghost notes show the channel's other tracks and clips, with a Ghosts switch in the snap menu, and their velocity and pan show in the graph lane. A long press on a ghost note opens its clip (#152, #154).
- Piano roll view. Notes show their names, the channel's name sits under the ruler, and you can zoom out to a whole long clip or in to single 1/128 steps (#154, #171, #175).
- Value bubble. Dragging a knob or slider shows its value in a bubble. Turn it off in Settings (#171).
- Note effects. Changing a note effect such as Transpose no longer leaves notes hanging (#140).
- Previews. A large audio file starts playing in the sample pickers and Songs while the rest of it loads (#131, #153).
- Song archives. Archives over 512 MiB open, import and unzip. A song in an archive's unzipped folder finds the archive's samples, and project ZIPs now include samples loaded in the Lemon samplers and samples from library folders (#128, #157, #168, #144).
- Android. Back closes the open panel first and then asks "Exit Honeydew?". Settings > Library folders adds folders from outside Honeydew to Songs and the pickers. Multi-core processing can be turned on in Settings and starts off (#147, #144, #145, #156).
- Android audio. Level meters no longer disappear at small buffer sizes, a 4096-frame buffer size is available, and the screen redraws at most 60 times a second while playing, leaving more time for audio (#145, #172).
- Recording. A new Input gain setting under Audio input, and Calibrate now works with quiet microphones in a noisy room (#129).
- Mixer. The master track shows its live peak in dB, in red above 0 dB, and the meters fall faster (#143, #135).
- Instruments. MiniSynth's slide starts fresh on each new note, a sample newly loaded into the DW Sampler plays at its own pitch on middle C, and the DW Sampler's Import opens .dwp and .sfz programs (#143, #102, #176).
- Lemon plugins. Updated to the latest Lemon Studio, Tesla's Convert to score puts its notes in a new clip at the playhead, and the type-in box has an OK button (#164, #167, #166).
- Drum editor. The Sample tab's waveform shows Sample Start and Stretch (#175).
- Audio clips. A clip's sound stays in line with the playlist when you change its Speed, thanks to tw12992 (#173).
- Save dialog. The estimated length follows tempo automation.
