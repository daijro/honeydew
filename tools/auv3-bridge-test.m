// DSP/state smoke test for the iOS bridge using an in-process AUv3 test unit on macOS.
// This exercises the shared Audio Unit API, not iOS extension discovery or UIKit.
// Run: tools/auv3-bridge-test.sh [output-directory]
#import "../src/plugins/auv3/bridge.h"
#import <Foundation/Foundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreFoundation/CoreFoundation.h>
#import <assert.h>
#import <math.h>


// A real AUAudioUnit subclass exercises the v3 pull-input/render path. Apple's
// legacy v2 wrapper requires AudioUnit connections and cannot stand in for it.
@interface HDTestUnit : AUAudioUnit {
    AUAudioUnitBusArray *inputs, *outputs;
    AUValue gain;
    BOOL instrument, note, multi;
}
@end
@implementation HDTestUnit
- (instancetype)initWithComponentDescription:(AudioComponentDescription)d options:(AudioComponentInstantiationOptions)o error:(NSError **)error {
    self = [super initWithComponentDescription:d options:o error:error];
    if (!self) return nil;
    BOOL mono = d.componentSubType == 'hdm1';
    multi = d.componentSubType == 'hdx1';
    AVAudioFormat *format = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:48000 channels:mono ? 1 : 2];
    instrument = d.componentType == kAudioUnitType_MusicDevice;
    inputs = [[AUAudioUnitBusArray alloc] initWithAudioUnit:self busType:AUAudioUnitBusTypeInput
        busses:instrument && !multi ? @[] : @[[[AUAudioUnitBus alloc] initWithFormat:format error:error]]];
    AUAudioUnitBus *main = [[AUAudioUnitBus alloc] initWithFormat:format error:error];
    if (mono) main.supportedChannelCounts = @[@1];
    outputs = [[AUAudioUnitBusArray alloc] initWithAudioUnit:self busType:AUAudioUnitBusTypeOutput
        busses:multi ? @[main, [[AUAudioUnitBus alloc] initWithFormat:format error:error]] : @[main]];
    AUParameter *p = [AUParameterTree createParameterWithIdentifier:@"gain" name:@"Gain" address:0x100000001ULL
        min:0 max:1 unit:kAudioUnitParameterUnit_LinearGain unitName:nil
        flags:kAudioUnitParameterFlag_IsReadable | kAudioUnitParameterFlag_IsWritable | kAudioUnitParameterFlag_CanRamp
        valueStrings:nil dependentParameters:nil];
    self.parameterTree = [AUParameterTree createTreeWithChildren:@[p]];
    __unsafe_unretained HDTestUnit *unit = self;
    self.parameterTree.implementorValueObserver = ^(AUParameter *p, AUValue value) { unit->gain = value; };
    self.parameterTree.implementorValueProvider = ^AUValue(AUParameter *p) { return unit->gain; };
    p.value = 0.5;
    return self;
}
- (BOOL)allocateRenderResourcesAndReturnError:(NSError **)error {
    if (multi && (outputs[1].enabled || inputs[0].enabled)) return NO;
    return [super allocateRenderResourcesAndReturnError:error];
}
- (AUAudioUnitBusArray *)inputBusses { return inputs; }
- (AUAudioUnitBusArray *)outputBusses { return outputs; }
- (NSDictionary *)fullState { return @{@"gain": @(gain)}; }
- (NSDictionary *)fullStateForDocument { return self.fullState; }
- (void)setFullState:(NSDictionary *)state { self.parameterTree.allParameters[0].value = [state[@"gain"] floatValue]; }
- (AUInternalRenderBlock)internalRenderBlock {
    __unsafe_unretained HDTestUnit *unit = self;
    return ^AUAudioUnitStatus(AudioUnitRenderActionFlags *flags, const AudioTimeStamp *time, AUAudioFrameCount count,
        NSInteger bus, AudioBufferList *data, const AURenderEvent *events, AURenderPullInputBlock pull) {
        if (!unit->instrument) {
            if (!pull) return kAudioUnitErr_NoConnection;
            OSStatus status = pull(flags, time, count, 0, data);
            if (status) return status;
        }
        for (const AURenderEvent *e = events; e; e = e->head.next) {
            if (e->head.eventType == AURenderEventParameter) unit->gain = e->parameter.value;
            if (e->head.eventType == AURenderEventMIDI && e->MIDI.length == 3) {
                if ((e->MIDI.data[0] & 0xF0) == 0x90) unit->note = e->MIDI.data[2] != 0;
                if ((e->MIDI.data[0] & 0xF0) == 0x80) unit->note = NO;
            }
        }
        for (UInt32 c = 0; c < data->mNumberBuffers; ++c)
            for (UInt32 i = 0; i < count; ++i) {
                float *value = &((float *)data->mBuffers[c].mData)[i];
                *value = unit->instrument ? (unit->note ? unit->gain : 0) : *value * unit->gain;
            }
        return noErr;
    };
}
@end

static id takeJSON(char *text) {
    assert(text);
    NSData *data = [NSData dataWithBytes:text length:strlen(text)];
    id value = [NSJSONSerialization JSONObjectWithData:data options:NSJSONReadingFragmentsAllowed error:nil];
    hd_au_free(text);
    assert(value);
    return value;
}
int main(void) {
    @autoreleasepool {
        AudioComponentDescription component = { kAudioUnitType_Effect, 'hdt1', 'Hdew', 0, 0 };
        [AUAudioUnit registerSubclass:HDTestUnit.class asComponentDescription:component name:@"Honeydew: Test" version:1];
        for (NSNumber *subtype in @[@((uint32_t)'hdi1'), @((uint32_t)'hdm1'), @((uint32_t)'hdx1')]) {
            AudioComponentDescription synth = { kAudioUnitType_MusicDevice, subtype.unsignedIntValue, 'Hdew', 0, 0 };
            [AUAudioUnit registerSubclass:HDTestUnit.class asComponentDescription:synth name:@"Honeydew: Instrument" version:1];
        }
        NSArray *discovered = takeJSON(hd_au_discover());
        for (NSDictionary *entry in discovered)
            assert(CFGetTypeID((__bridge CFTypeRef)entry[@"instrument"]) == CFBooleanGetTypeID());
        BOOL found = NO;
        for (NSDictionary *entry in discovered)
            if ([entry[@"subtype"] unsignedIntValue] == 'hdt1' && [entry[@"manufacturer"] unsignedIntValue] == 'Hdew') found = YES;
        assert(found);
        void *unit = hd_au_create(kAudioUnitType_Effect, 'hdt1', 'Hdew');
        assert(unit);
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:15];
        int status;
        while ((status = hd_au_poll(unit, 48000)) == 0 && deadline.timeIntervalSinceNow > 0)
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, false);
        if (status != 1) { NSLog(@"Load error: %@", takeJSON(hd_au_error(unit))); return 1; }
        NSArray *parameters = takeJSON(hd_au_parameters(unit));
        assert(parameters.count > 0);
        for (NSDictionary *parameter in parameters) {
            assert(CFGetTypeID((__bridge CFTypeRef)parameter[@"writable"]) == CFBooleanGetTypeID());
            assert(CFGetTypeID((__bridge CFTypeRef)parameter[@"automatable"]) == CFBooleanGetTypeID());
        }
        size_t stateLength = 0;
        uint8_t *state = hd_au_state(unit, &stateLength);
        assert(state && stateLength);
        NSData *bytes = [NSData dataWithBytes:state length:stateLength];
        hd_au_free((char *)state);
        float original = hd_au_value(unit, 0);
        hd_au_set_value(unit, 0, original > 0.5 ? 0.2 : 0.8);
        assert(fabsf(hd_au_value(unit, 0) - original) > 0.01);
        assert(hd_au_load_state(unit, bytes.bytes, bytes.length));
        assert(fabsf(hd_au_value(unit, 0) - original) < 0.001);
        assert(!hd_au_load_state(unit, (const uint8_t *)"invalid", 7));
        // Exercise splitting a buffer larger than maximumFramesToRender and sample clocks.
        hd_au_parameter(unit, 0, 0.25, 0);
        float left[8192] = {1}, right[8192] = {1};
        hd_au_transport(unit, 123, 2, 0, 48000, 4, 4, 1);
        if (!hd_au_render(unit, left, right, 8192, 1)) { NSLog(@"Render error: %@", takeJSON(hd_au_error(unit))); return 1; }
        double energy = 0;
        for (int i = 0; i < 8192; ++i) {
            assert(isfinite(left[i]) && isfinite(right[i]));
            energy += fabs(left[i]) + fabs(right[i]);
        }
        assert(energy > 0);
        assert(fabsf(left[0] - 0.25f) < 0.001f);
        hd_au_reset(unit);
        assert(hd_au_poll(unit, 44100) == 1);
        hd_au_offline(unit, 1);
        float l[32] = {0}, r[32] = {0};
        assert(hd_au_render(unit, l, r, 32, 1));
        hd_au_destroy(unit);
        component.componentType = kAudioUnitType_MusicDevice;
        for (NSNumber *subtype in @[@((uint32_t)'hdi1'), @((uint32_t)'hdm1'), @((uint32_t)'hdx1')]) {
            component.componentSubType = subtype.unsignedIntValue;
            BOOL listedAsInstrument = NO;
            for (NSDictionary *entry in takeJSON(hd_au_discover()))
                if ([entry[@"subtype"] unsignedIntValue] == component.componentSubType && [entry[@"instrument"] boolValue]) listedAsInstrument = YES;
            assert(listedAsInstrument);
            unit = hd_au_create(kAudioUnitType_MusicDevice, component.componentSubType, 'Hdew');
            deadline = [NSDate dateWithTimeIntervalSinceNow:15];
            while ((status = hd_au_poll(unit, 48000)) == 0 && deadline.timeIntervalSinceNow > 0)
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, false);
            if (status != 1) { NSLog(@"Synth load error: %@", takeJSON(hd_au_error(unit))); return 1; }
            const uint8_t on[] = {0x90, 60, 100}, off[] = {0x80, 60, 0};
            hd_au_midi(unit, 0, on, 3);
            assert(hd_au_render(unit, l, r, 32, 0));
            assert(l[0] > 0 && r[0] > 0);
            assert(l[0] == r[0]); // A mono synth is duplicated to both rack channels.
            hd_au_midi(unit, 0, off, 3);
            assert(hd_au_render(unit, l, r, 32, 0));
            assert(l[0] == 0 && r[0] == 0);
            hd_au_destroy(unit);
        }
        // Dropping while the async creation callback is pending must be safe.
        unit = hd_au_create(kAudioUnitType_Effect, 'hdt1', 'Hdew');
        hd_au_destroy(unit);
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, false);
        puts("AU bridge DSP/state smoke test passed");
    }
    return 0;
}
