//! End-to-end registration, save/load and rendering contracts for the new rack FX.
use banaforge::{flm, params, panels, song::{ModuleInstance, Song}};
use banaforge_audio::{banana_power as b, AudioEffect, ProcessContext};
#[test]
fn banana_power_modules_have_panels_and_round_trip() {
    let mut song = Song::default();
    for (i,id) in b::IDS.into_iter().enumerate() {
        let spec=params::module_spec(id).expect("selectable module spec");
        assert_eq!(spec.display_name,b::NAMES[i]);
        assert!(panels::panel_for(id).is_some(),"native card missing");
        assert_eq!(spec.params.iter().map(|p|p.default).collect::<Vec<_>>(),b::defaults(id));
        let mut module=ModuleInstance::new(id,i as i32+1);
        module.params[1]=0.8;
        if id==b::GROSS_BEAT { module.params[b::TIME_POINTS+4]=0.75; module.params[b::VOLUME_POINTS+7]=0.25; }
        song.channels[0].rack.modules.push(module);
        let mut fx=AudioEffect::new(id,48000.0).unwrap();
        fx.set_parameter(1,0.8).unwrap();
        let mut l=vec![0.2;512];let mut r=l.clone();
        fx.process(&mut l,&mut r,ProcessContext{sample_rate:48000.0,tempo_bpm:120.0,start_beat:0.0,playing:true}).unwrap();
        assert!(l.iter().all(|v|v.is_finite()));
    }
    let loaded=flm::read(&flm::write(&song).unwrap()).unwrap();
    for module in &song.channels[0].rack.modules {
        let restored=loaded.channels[0].rack.modules.iter().find(|m|m.type_id==module.type_id).unwrap();
        assert_eq!(restored.params,module.params);
    }
}
