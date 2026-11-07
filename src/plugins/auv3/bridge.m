// AUv3 hosting for Honeydew. ARC owns Objective-C objects; Rust owns each HDUnit.
#import "bridge.h"
#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <TargetConditionals.h>
#import <CoreAudioKit/CoreAudioKit.h>
#if TARGET_OS_IOS
#import <UIKit/UIKit.h>
#else
#import <AppKit/AppKit.h>
#endif
#import <stdatomic.h>
#import <math.h>

#define HD_FRAMES 4096

typedef struct { UInt32 count; AudioBuffer buffers[2]; } HDBuffers;
static char *json(id object) {
    NSData *data = [NSJSONSerialization dataWithJSONObject:object options:NSJSONWritingFragmentsAllowed error:nil];
    if (!data) return NULL;
    char *result = malloc(data.length + 1);
    if (!result) return NULL;
    memcpy(result, data.bytes, data.length); result[data.length] = 0;
    return result;
}
void hd_au_free(char *s) { free(s); }

@interface HDUnit : NSObject {
@public
    AVAudioUnit *av;
    AUAudioUnit *au;
    NSString *error;
    NSArray<AUParameter *> *parameters;
    AUParameterObserverToken observer;
    _Atomic(uint32_t) *changes;
    NSData *pendingState;
    AURenderBlock render;
    AURenderPullInputBlock pull;
    AUScheduleMIDIEventBlock midi;
    AUScheduleParameterBlock parameter;
    uint32_t rate;
    UInt32 outputChannels;
    BOOL instrument;
    OSStatus renderStatus;
    BOOL ready, failed, offline, editorRequested, editorUnavailable;
    uint64_t editorGeneration;
#if TARGET_OS_IOS
    UIViewController *editor;
#else
    NSViewController *editor;
    NSWindow *editorWindow;
#endif
    double sampleTime, tempo, beat, bar, frame;
    uint32_t numerator, denominator;
    BOOL playing;
    float input[2][HD_FRAMES], output[2][HD_FRAMES];
}
@end
@implementation HDUnit
- (void)dealloc {
    if (observer) [au.parameterTree removeParameterObserver:observer];
    free(changes);
}
@end

char *hd_au_discover(void) {
    @autoreleasepool {
        NSMutableArray *result = [NSMutableArray array];
        for (NSNumber *type in @[@(kAudioUnitType_MusicDevice), @(kAudioUnitType_Effect), @(kAudioUnitType_MusicEffect)]) {
            AudioComponentDescription query = { .componentType = type.unsignedIntValue };
            for (AVAudioUnitComponent *c in [[AVAudioUnitComponentManager sharedAudioUnitComponentManager] componentsMatchingDescription:query]) {
                AudioComponentDescription d = c.audioComponentDescription;
                if (!(d.componentFlags & kAudioComponentFlag_IsV3AudioUnit)) continue;
                [result addObject:@{@"type": @(d.componentType), @"subtype": @(d.componentSubType),
                    @"manufacturer": @(d.componentManufacturer), @"name": c.name ?: @"Audio Unit",
                    @"vendor": c.manufacturerName ?: @"", @"instrument": d.componentType == kAudioUnitType_MusicDevice ? @YES : @NO}];
            }
        }
        return json(result);
    }
}

void *hd_au_create(uint32_t type, uint32_t subtype, uint32_t manufacturer) {
    HDUnit *u = [HDUnit new];
    u->instrument = type == kAudioUnitType_MusicDevice;
    u->tempo = 120; u->numerator = u->denominator = 4;
    AudioComponentDescription d = { type, subtype, manufacturer, 0, 0 };
    // No blocking semaphore or nested application run loop. Completion is adopted on main.
    __weak HDUnit *weak = u;
    [AVAudioUnit instantiateWithComponentDescription:d options:kAudioComponentInstantiation_LoadOutOfProcess
        completionHandler:^(AVAudioUnit *unit, NSError *e) {
        dispatch_async(dispatch_get_main_queue(), ^{
            HDUnit *strong = weak;
            if (!strong) return;
            strong->av = unit; strong->au = unit.AUAudioUnit;
            if (!unit) { strong->failed = YES; strong->error = e.localizedDescription ?: @"AUv3 could not load"; }
        });
    }];
    return (__bridge_retained void *)u;
}

static BOOL restore(HDUnit *u, NSData *data) {
    if (!data.length) return YES;
    id state = [NSPropertyListSerialization propertyListWithData:data options:NSPropertyListImmutable format:nil error:nil];
    if (![state isKindOfClass:[NSDictionary class]]) return NO;
    @try { u->au.fullState = state; return YES; }
    @catch (NSException *e) { u->error = e.reason; return NO; }
}

int hd_au_poll(void *handle, uint32_t rate) {
    HDUnit *u = (__bridge HDUnit *)handle;
    if (u->failed) return -1;
    if (!u->au) return 0;
    if (u->ready && u->rate == rate) return 1;
    @try {
        [u->au deallocateRenderResources]; u->ready = NO;
        if (u->pendingState && !restore(u, u->pendingState)) {
            u->failed = YES; u->error = @"AUv3 rejected the saved state"; return -1;
        }
        u->pendingState = nil;
        AVAudioFormat *format = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:rate channels:2];
        NSError *e = nil;
        if (!u->au.outputBusses.count || (!u->instrument && (u->au.outputBusses.count != 1 || u->au.inputBusses.count > 1))) {
            u->failed = YES; u->error = @"This AUv3 requires an unsupported audio layout"; return -1;
        }
        // Synths render their main output. Auxiliary outputs and audio inputs
        // are unconnected; they must not prevent a multi-output synth loading.
        u->au.outputBusses[0].enabled = YES;
        if (u->instrument) {
            for (NSUInteger i = 1; i < u->au.outputBusses.count; ++i) u->au.outputBusses[i].enabled = NO;
            for (AUAudioUnitBus *bus in u->au.inputBusses) bus.enabled = NO;
        }
        BOOL outputOK = [u->au.outputBusses[0] setFormat:format error:&e];
        u->outputChannels = 2;
        if (!outputOK && u->instrument) {
            AVAudioFormat *mono = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:rate channels:1];
            outputOK = [u->au.outputBusses[0] setFormat:mono error:&e];
            if (outputOK) u->outputChannels = 1;
        }
        if (!outputOK || (!u->instrument && u->au.inputBusses.count && ![u->au.inputBusses[0] setFormat:format error:&e])) {
            u->failed = YES; u->error = e.localizedDescription ?: @"AUv3 does not support this audio format"; return -1;
        }
        u->au.maximumFramesToRender = HD_FRAMES;
        u->au.renderingOffline = u->offline;
        // Weak capture prevents the unit's cached blocks from retaining their owner.
        __weak HDUnit *weak = u;
        u->au.musicalContextBlock = ^BOOL(double *tempo, double *num, NSInteger *den, double *beat, NSInteger *next, double *bar) {
            HDUnit *s = weak; if (!s) return NO;
            if (tempo) *tempo = s->tempo; if (num) *num = s->numerator; if (den) *den = s->denominator;
            if (beat) *beat = s->beat; if (bar) *bar = s->bar;
            if (next) *next = (NSInteger)((ceil(s->beat) - s->beat) * 60.0 / s->tempo * s->rate);
            return YES;
        };
        u->au.transportStateBlock = ^BOOL(AUHostTransportStateFlags *flags, double *position, double *start, double *end) {
            HDUnit *s = weak; if (!s) return NO;
            if (flags) *flags = s->playing ? AUHostTransportStateMoving : 0;
            if (position) *position = s->frame; if (start) *start = 0; if (end) *end = 0;
            return YES;
        };
        if (![u->au allocateRenderResourcesAndReturnError:&e]) {
            u->failed = YES; u->error = e.localizedDescription ?: @"AUv3 could not allocate audio resources"; return -1;
        }
        u->render = u->au.renderBlock;
        u->midi = u->au.scheduleMIDIEventBlock;
        u->parameter = u->au.scheduleParameterBlock;
        // Unretained pointer: the owner keeps this block; it cannot outlive a synchronous render call.
        __unsafe_unretained HDUnit *source = u;
        u->pull = ^AUAudioUnitStatus(AudioUnitRenderActionFlags *flags, const AudioTimeStamp *time,
                                    AUAudioFrameCount count, NSInteger bus, AudioBufferList *data) {
            (void)flags; (void)time;
            if (count > HD_FRAMES || data->mNumberBuffers > 2 || !data->mNumberBuffers || (!source->instrument && (bus != 0 || data->mNumberBuffers != 2))) return kAudioUnitErr_FormatNotSupported;
            for (UInt32 c = 0; c < data->mNumberBuffers; ++c) {
                if (data->mBuffers[c].mData) memcpy(data->mBuffers[c].mData, source->input[c], count * sizeof(float));
                else data->mBuffers[c].mData = source->input[c];
                data->mBuffers[c].mDataByteSize = count * sizeof(float);
            }
            return noErr;
        };
        if (!u->parameters) {
            u->parameters = [u->au.parameterTree.allParameters sortedArrayUsingComparator:^NSComparisonResult(AUParameter *a, AUParameter *b) {
                return a.address < b.address ? NSOrderedAscending : a.address > b.address ? NSOrderedDescending : NSOrderedSame;
            }] ?: @[];
            u->changes = calloc(u->parameters.count, sizeof(*u->changes));
            if (u->parameters.count && !u->changes) {
                u->failed = YES; u->error = @"Not enough memory for AUv3 parameters"; return -1;
            }
            for (NSUInteger i = 0; i < u->parameters.count; ++i) atomic_init(&u->changes[i], UINT32_MAX);
            // AUParameter.value may cache a UI value and need not reflect scheduled
            // automation. Observe editor changes instead of polling that cached value.
            u->observer = [u->au.parameterTree tokenByAddingParameterObserver:^(AUParameterAddress address, AUValue value) {
                HDUnit *s = weak; if (!s) return;
                NSUInteger lo = 0, hi = s->parameters.count;
                while (lo < hi) {
                    NSUInteger mid = lo + (hi - lo) / 2;
                    if (s->parameters[mid].address < address) lo = mid + 1; else hi = mid;
                }
                if (lo >= s->parameters.count || s->parameters[lo].address != address) return;
                AUParameter *p = s->parameters[lo];
                float normalized = p.maxValue > p.minValue ? (value - p.minValue) / (p.maxValue - p.minValue) : 0;
                uint32_t bits; memcpy(&bits, &normalized, sizeof(bits));
                atomic_store_explicit(&s->changes[lo], bits, memory_order_release);
            }];
        }
        u->rate = rate; u->sampleTime = 0; u->ready = YES;
        return 1;
    } @catch (NSException *e) { u->failed = YES; u->error = e.reason; return -1; }
}

char *hd_au_error(void *h) { HDUnit *u = (__bridge HDUnit *)h; return json(u->error ?: (u->renderStatus ? [NSString stringWithFormat:@"AUv3 render error %d", (int)u->renderStatus] : @"AUv3 failed")); }
char *hd_au_parameters(void *h) {
    HDUnit *u = (__bridge HDUnit *)h;
    NSMutableArray *result = [NSMutableArray array];
    for (AUParameter *p in u->parameters) {
        [result addObject:@{@"address": [NSString stringWithFormat:@"%llu", (unsigned long long)p.address],
            @"name": p.displayName ?: p.identifier, @"min": @(p.minValue), @"max": @(p.maxValue),
            @"value": @(p.value), @"writable": (p.flags & kAudioUnitParameterFlag_IsWritable) ? @YES : @NO,
            @"automatable": (p.flags & kAudioUnitParameterFlag_NonRealTime) == 0 ? @YES : @NO}];
    }
    return json(result);
}
float hd_au_value(void *h, uint32_t index) {
    HDUnit *u = (__bridge HDUnit *)h;
    if (index >= u->parameters.count) return 0;
    AUParameter *p = u->parameters[index];
    return p.maxValue > p.minValue ? (p.value - p.minValue) / (p.maxValue - p.minValue) : 0;
}
float hd_au_take_value(void *h, uint32_t index) {
    HDUnit *u = (__bridge HDUnit *)h;
    if (index >= u->parameters.count) return NAN;
    uint32_t bits = atomic_exchange_explicit(&u->changes[index], UINT32_MAX, memory_order_acquire);
    float value; memcpy(&value, &bits, sizeof(value)); return value;
}
void hd_au_parameter(void *h, uint32_t index, float value, uint32_t offset) {
    HDUnit *u = (__bridge HDUnit *)h;
    if (!u->ready || !u->parameter || index >= u->parameters.count) return;
    AUParameter *p = u->parameters[index];
    if (!(p.flags & kAudioUnitParameterFlag_IsWritable) || !isfinite(value)) return;
    u->parameter((AUEventSampleTime)u->sampleTime + offset, 0, p.address,
                 p.minValue + fmaxf(0, fminf(1, value)) * (p.maxValue - p.minValue));
}
void hd_au_set_value(void *h, uint32_t index, float value) {
    HDUnit *u = (__bridge HDUnit *)h;
    if (!u->ready || index >= u->parameters.count || !isfinite(value)) return;
    AUParameter *p = u->parameters[index];
    if (p.flags & kAudioUnitParameterFlag_IsWritable)
        [p setValue:p.minValue + fmaxf(0, fminf(1, value)) * (p.maxValue - p.minValue) originator:u->observer];
}
void hd_au_midi(void *h, uint32_t offset, const uint8_t *bytes, uint32_t count) {
    HDUnit *u = (__bridge HDUnit *)h;
    if (u->ready && u->midi) u->midi((AUEventSampleTime)u->sampleTime + offset, 0, count, bytes);
}
int hd_au_render(void *h, float *left, float *right, uint32_t frames, int effect) {
    HDUnit *u = (__bridge HDUnit *)h;
    if (!u->ready || u->failed) return 0;
    for (uint32_t offset = 0; offset < frames;) {
        uint32_t n = MIN(HD_FRAMES, frames - offset);
        if (effect) { memcpy(u->input[0], left + offset, n * sizeof(float)); memcpy(u->input[1], right + offset, n * sizeof(float)); }
        else { memset(u->input, 0, sizeof(u->input)); }
        HDBuffers buffers = {u->outputChannels, {{1, n * sizeof(float), u->output[0]}, {1, n * sizeof(float), u->output[1]}}};
        AudioTimeStamp time = { .mSampleTime = u->sampleTime, .mFlags = kAudioTimeStampSampleTimeValid };
        AudioUnitRenderActionFlags flags = 0;
        OSStatus status = u->render(&flags, &time, n, 0, (AudioBufferList *)&buffers, u->pull);
        if (status != noErr) { u->renderStatus = status; u->failed = YES; return 0; }
        if (buffers.count != u->outputChannels || !buffers.buffers[0].mData || (u->outputChannels == 2 && !buffers.buffers[1].mData)) {
            u->renderStatus = kAudioUnitErr_InvalidPropertyValue; u->failed = YES; return 0;
        }
        if (flags & kAudioUnitRenderAction_OutputIsSilence) {
            memset(left + offset, 0, n * sizeof(float));
            memset(right + offset, 0, n * sizeof(float));
        } else {
        // Units may replace mData with their own output buffers.
        memcpy(left + offset, buffers.buffers[0].mData, n * sizeof(float));
        memcpy(right + offset, buffers.buffers[u->outputChannels == 1 ? 0 : 1].mData, n * sizeof(float));
        }
        u->sampleTime += n; u->frame += n;
        u->beat += n * u->tempo / (60.0 * u->rate);
        offset += n;
    }
    return 1;
}
void hd_au_reset(void *h) {
    HDUnit *u = (__bridge HDUnit *)h;
    [u->au reset]; u->playing = NO;
    if (u->ready && u->midi) {
        const uint8_t soundOff[] = {0xB0, 120, 0}, notesOff[] = {0xB0, 123, 0}, sustainOff[] = {0xB0, 64, 0};
        u->midi((AUEventSampleTime)u->sampleTime, 0, 3, sustainOff);
        u->midi((AUEventSampleTime)u->sampleTime, 0, 3, soundOff);
        u->midi((AUEventSampleTime)u->sampleTime, 0, 3, notesOff);
    }
}
void hd_au_transport(void *h, double tempo, double beat, double bar, double frame, uint32_t num, uint32_t den, int playing) {
    HDUnit *u = (__bridge HDUnit *)h;
    u->tempo = fmax(1, tempo); u->beat = beat; u->bar = bar; u->frame = frame;
    u->numerator = num; u->denominator = den; u->playing = playing;
}
uint8_t *hd_au_state(void *h, size_t *length) {
    HDUnit *u = (__bridge HDUnit *)h;
    *length = 0;
    @try {
        NSData *data = u->pendingState;
        if (u->ready && !u->failed) data = [NSPropertyListSerialization dataWithPropertyList:u->au.fullStateForDocument ?: u->au.fullState ?: @{}
            format:NSPropertyListBinaryFormat_v1_0 options:0 error:nil];
        if (!data.length) return NULL;
        uint8_t *bytes = malloc(data.length);
        if (!bytes) return NULL;
        memcpy(bytes, data.bytes, data.length);
        *length = data.length;
        return bytes;
    } @catch (NSException *e) { return NULL; }
}
int hd_au_load_state(void *h, const uint8_t *bytes, size_t length) {
    HDUnit *u = (__bridge HDUnit *)h;
    NSData *data = [NSData dataWithBytes:bytes length:length];
    id state = [NSPropertyListSerialization propertyListWithData:data options:NSPropertyListImmutable format:nil error:nil];
    if (![state isKindOfClass:[NSDictionary class]]) return 0;
    if (!u->ready) { u->pendingState = data; return 1; }
    return restore(u, data);
}
uint32_t hd_au_latency(void *h) { HDUnit *u = (__bridge HDUnit *)h; return (uint32_t)(u->au.latency * u->rate); }
void hd_au_offline(void *h, int value) { HDUnit *u = (__bridge HDUnit *)h; u->offline = value; u->au.renderingOffline = value; }

int hd_au_has_editor(void *h) {
    HDUnit *u = (__bridge HDUnit *)h;
    return u->ready && !u->failed && !u->editorUnavailable;
}
#if TARGET_OS_IOS
// The host owns this toolbar outside the plugin's view hierarchy. A plugin can
// replace its own navigation items without hiding the way back to the DAW.
@interface HDPluginEditorController : UIViewController
@property(nonatomic, strong) UIViewController *pluginController;
@property(nonatomic, copy) NSString *pluginTitle;
@property(nonatomic, strong) UINavigationBar *hostNavigationBar;
@end
@implementation HDPluginEditorController
- (void)loadView {
    UIView *root = [UIView new];
    root.backgroundColor = UIColor.systemBackgroundColor;
    self.view = root;
    UINavigationBar *bar = [[UINavigationBar alloc] initWithFrame:CGRectZero];
    bar.translatesAutoresizingMaskIntoConstraints = NO;
    UINavigationItem *item = [[UINavigationItem alloc] initWithTitle:self.pluginTitle ?: @"Plugin"];
    item.leftBarButtonItem = [[UIBarButtonItem alloc] initWithTitle:@"Back to Honeydew"
        style:UIBarButtonItemStylePlain target:self action:@selector(backToHoneydew:)];
    item.leftBarButtonItem.accessibilityIdentifier = @"honeydew.auv3.back";
    bar.items = @[item];
    self.hostNavigationBar = bar;
    [root addSubview:bar];
    UIViewController *plugin = self.pluginController;
    [self addChildViewController:plugin];
    plugin.view.translatesAutoresizingMaskIntoConstraints = NO;
    plugin.view.clipsToBounds = YES;
    [root addSubview:plugin.view];
    [NSLayoutConstraint activateConstraints:@[
        [bar.topAnchor constraintEqualToAnchor:root.safeAreaLayoutGuide.topAnchor],
        [bar.leadingAnchor constraintEqualToAnchor:root.safeAreaLayoutGuide.leadingAnchor],
        [bar.trailingAnchor constraintEqualToAnchor:root.safeAreaLayoutGuide.trailingAnchor],
        [bar.heightAnchor constraintEqualToConstant:44],
        [plugin.view.topAnchor constraintEqualToAnchor:bar.bottomAnchor],
        [plugin.view.leadingAnchor constraintEqualToAnchor:root.safeAreaLayoutGuide.leadingAnchor],
        [plugin.view.trailingAnchor constraintEqualToAnchor:root.safeAreaLayoutGuide.trailingAnchor],
        [plugin.view.bottomAnchor constraintEqualToAnchor:root.safeAreaLayoutGuide.bottomAnchor],
    ]];
    [plugin didMoveToParentViewController:self];
}
- (void)backToHoneydew:(id)sender {
    (void)sender;
    [self dismissViewControllerAnimated:YES completion:nil];
}
@end
static UIViewController *topController(UIWindow *window) {
    UIViewController *vc = window.rootViewController;
    while (vc.presentedViewController) vc = vc.presentedViewController;
    return vc;
}
static UIViewController *presenter(void) {
    for (UIScene *scene in UIApplication.sharedApplication.connectedScenes) {
        if (scene.activationState != UISceneActivationStateForegroundActive || ![scene isKindOfClass:UIWindowScene.class]) continue;
        for (UIWindow *window in ((UIWindowScene *)scene).windows)
            if (window.isKeyWindow) return topController(window);
    }
    // winit also supports the application-delegate lifecycle without UIScene.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    for (UIWindow *window in UIApplication.sharedApplication.windows)
        if (window.isKeyWindow) return topController(window);
#pragma clang diagnostic pop
    return nil;
}
int hd_au_open_editor(void *h) {
    HDUnit *u = (__bridge HDUnit *)h;
    if (!u->ready || u->failed || !presenter()) return 0;
    if (u->editorRequested || u->editor.presentingViewController) return 1;
    u->editor = nil;
    u->editorRequested = YES;
    uint64_t generation = ++u->editorGeneration;
    __weak HDUnit *weak = u;
    [u->au requestViewControllerWithCompletionHandler:^(UIViewController *vc) {
        dispatch_async(dispatch_get_main_queue(), ^{
            HDUnit *s = weak;
            if (!s || generation != s->editorGeneration) return;
            s->editorRequested = NO;
            UIViewController *parent = presenter();
            if (!vc) { s->editorUnavailable = YES; return; }
            if (!parent) return; // Generic parameter controls remain available in the rack.
            HDPluginEditorController *container = [HDPluginEditorController new];
            container.pluginController = vc;
            container.pluginTitle = s->au.componentName;
            container.modalPresentationStyle = UIModalPresentationPageSheet;
            CGSize preferred = vc.preferredContentSize;
            if (preferred.width > 0 && preferred.height > 0)
                container.preferredContentSize = CGSizeMake(preferred.width, preferred.height + 44);
            s->editor = container;
            [parent presentViewController:container animated:YES completion:nil];
        });
    }];
    return 1;
}
void hd_au_close_editor(void *h) {
    HDUnit *u = (__bridge HDUnit *)h;
    ++u->editorGeneration; u->editorRequested = NO;
    [u->editor dismissViewControllerAnimated:YES completion:nil]; u->editor = nil;
}
int hd_au_editor_open(void *h) { HDUnit *u = (__bridge HDUnit *)h; return u->editor.presentingViewController ? 2 : u->editorRequested ? 1 : 0; }
#else
static NSWindow *editorParentWindow(void) {
    NSWindow *parent = NSApp.mainWindow ?: NSApp.keyWindow;
    while (parent.parentWindow) parent = parent.parentWindow;
    return parent;
}
static void attachEditorWindow(NSWindow *window, NSWindow *parent) {
    if (!parent || parent == window || window.parentWindow == parent) return;
    [window.parentWindow removeChildWindow:window];
    // Child ordering keeps the editor above the DAW without stealing keyboard focus
    // when a user clicks Honeydew's piano, and follows normal app hiding/minimizing.
    [parent addChildWindow:window ordered:NSWindowAbove];
}
int hd_au_open_editor(void *h) {
    HDUnit *u = (__bridge HDUnit *)h;
    if (!u->ready || u->failed || !NSThread.isMainThread) return 0;
    if (u->editorWindow) {
        attachEditorWindow(u->editorWindow, editorParentWindow());
        [u->editorWindow makeKeyAndOrderFront:nil]; return 1;
    }
    if (u->editorRequested) return 1;
    u->editorRequested = YES;
    uint64_t generation = ++u->editorGeneration;
    __weak HDUnit *weak = u;
    __weak NSWindow *parentWindow = editorParentWindow();
    [u->au requestViewControllerWithCompletionHandler:^(NSViewController *vc) {
        dispatch_async(dispatch_get_main_queue(), ^{
            HDUnit *s = weak;
            if (!s || generation != s->editorGeneration) return;
            s->editorRequested = NO;
            if (!vc) { s->editorUnavailable = YES; return; }
            s->editor = vc;
            NSSize size = vc.preferredContentSize;
            if (size.width <= 0 || size.height <= 0) size = vc.view.frame.size;
            if (size.width <= 0 || size.height <= 0) size = NSMakeSize(900, 600);
            NSWindow *window = [[NSWindow alloc] initWithContentRect:NSMakeRect(100, 100, size.width, size.height)
                styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable
                backing:NSBackingStoreBuffered defer:NO];
            window.releasedWhenClosed = NO;
            window.title = s->au.componentName ?: @"Audio Unit";
            window.contentViewController = vc;
            [window setContentSize:size];
            [window center];
            s->editorWindow = window;
            attachEditorWindow(window, parentWindow);
            [window makeKeyAndOrderFront:nil];
        });
    }];
    return 1;
}
void hd_au_close_editor(void *h) {
    HDUnit *u = (__bridge HDUnit *)h;
    ++u->editorGeneration; u->editorRequested = NO;
    [u->editorWindow.parentWindow removeChildWindow:u->editorWindow];
    [u->editorWindow orderOut:nil];
    [u->editorWindow close];
    u->editorWindow.contentViewController = nil;
    u->editorWindow = nil; u->editor = nil;
}
int hd_au_editor_open(void *h) {
    HDUnit *u = (__bridge HDUnit *)h;
    return u->editorWindow.visible ? 2 : u->editorRequested ? 1 : 0;
}

#endif
void hd_au_destroy(void *h) {
    HDUnit *u = (__bridge_transfer HDUnit *)h;
    hd_au_close_editor((__bridge void *)u);
    [u->au deallocateRenderResources];
}
