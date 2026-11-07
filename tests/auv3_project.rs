//! AUv3 modules survive project round trips on platforms without the extension.
use honeydew::song::plugin::{PLUGIN_EFFECT, PLUGIN_INSTRUMENT, PluginParam, PluginRef};
use honeydew::song::{ChannelKind, Song};
use honeydew::{flm, plugins};

#[test]
fn missing_auv3_preserves_identity_parameters_and_binary_state() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    for (kind, component) in [(PLUGIN_INSTRUMENT, "61756D75"), (PLUGIN_EFFECT, "61756678")] {
        let rack = &mut song.channels[1].rack;
        let id = rack.add(kind);
        let module = rack.get_mut(id).unwrap();
        module.params.push(0.73);
        module.file.plugin = Some(Box::new(PluginRef {
            format: "auv3".into(),
            id: format!("{component}-54455354-48446577"),
            name: "Missing AUv3".into(),
            vendor: "Test".into(),
            path: String::new(),
            state: vec![0, 255, 3, 0, 128].into(),
            params: vec![PluginParam {
                id: 0xFEDC1234,
                name: "Gain".into(),
                default: 0.5,
            }],
        }));
    }
    let bytes = flm::write(&song).unwrap();
    let mut back = flm::read(&bytes).unwrap();
    plugins::sync(&back);
    plugins::store_states(&mut back);
    for (original, restored) in song.channels[1]
        .rack
        .modules
        .iter()
        .zip(&back.channels[1].rack.modules)
    {
        let a = original.file.plugin.as_ref().unwrap();
        let b = restored.file.plugin.as_ref().unwrap();
        assert_eq!(a, b);
        assert_eq!(a.state, b.state);
        assert_eq!(original.params, restored.params);
        assert_eq!(b.format_label(), "AUv3");
    }
    assert_eq!(flm::write(&back).unwrap(), bytes);
}
