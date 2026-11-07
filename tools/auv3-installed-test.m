// Optional macOS integration test against an installed AUv3 by name.
// Creates its own instance; never writes the app's or plugin's user files.
#import "../src/plugins/auv3/bridge.h"
#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>
#import <CoreFoundation/CoreFoundation.h>
#import <math.h>
#import <stdio.h>

#define CHECK(x, message) do { if (!(x)) { fprintf(stderr, "FAIL: %s\n", message); return 1; } } while (0)
static id takeJSON(char *text) {
    if (!text) return nil;
    NSData *data = [NSData dataWithBytes:text length:strlen(text)];
    id value = [NSJSONSerialization JSONObjectWithData:data options:NSJSONReadingFragmentsAllowed error:nil];
    hd_au_free(text); return value;
}
static void tick(void) {
    NSEvent *event;
    while ((event = [NSApp nextEventMatchingMask:NSEventMaskAny untilDate:NSDate.date inMode:NSDefaultRunLoopMode dequeue:YES]))
        [NSApp sendEvent:event];
    CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, false);
}
int main(int argc, const char **argv) {
    @autoreleasepool {
        CHECK(argc >= 2, "supply an installed AUv3 name, e.g. 'Super J8'");
        NSApplication *app = NSApplication.sharedApplication;
        [app finishLaunching];
        BOOL testEditor = argc > 2 && strcmp(argv[2], "--editor") == 0;
        NSWindow *host = nil;
        if (testEditor) {
            [app setActivationPolicy:NSApplicationActivationPolicyRegular];
            if (@available(macOS 14.0, *)) [app activate];
            host = [[NSWindow alloc] initWithContentRect:NSMakeRect(40, 40, 1100, 800)
                styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable
                backing:NSBackingStoreBuffered defer:NO];
            host.releasedWhenClosed = NO;
            host.title = @"Honeydew editor ordering test";
            [host makeKeyAndOrderFront:nil];
            [host makeMainWindow];
            NSDate *focusDeadline = [NSDate dateWithTimeIntervalSinceNow:3];
            while (!app.mainWindow && focusDeadline.timeIntervalSinceNow > 0) tick();
            CHECK(app.mainWindow == host, "test DAW window did not become main");
        }
        NSString *name = [NSString stringWithUTF8String:argv[1]];
        NSDictionary *component = nil;
        for (NSDictionary *entry in takeJSON(hd_au_discover())) {
            printf("Installed: %s (%s)\n", [entry[@"name"] UTF8String], [entry[@"instrument"] boolValue] ? "instrument" : "effect");
            if ([entry[@"name"] rangeOfString:name options:NSCaseInsensitiveSearch].location != NSNotFound) component = entry;
        }
        CHECK(component != nil, "AUv3 was not discovered");
        void *unit = hd_au_create([component[@"type"] unsignedIntValue], [component[@"subtype"] unsignedIntValue], [component[@"manufacturer"] unsignedIntValue]);
        CHECK(unit != NULL, "could not create AUv3 handle");
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:30];
        int status = 0;
        while ((status = hd_au_poll(unit, 48000)) == 0 && deadline.timeIntervalSinceNow > 0) tick();
        if (status != 1) { fprintf(stderr, "Load: %s\n", [[takeJSON(hd_au_error(unit)) description] UTF8String]); hd_au_destroy(unit); return 1; }
        printf("Loaded: %s, %lu parameters\n", [component[@"name"] UTF8String], (unsigned long)[takeJSON(hd_au_parameters(unit)) count]);
        BOOL instrument = [component[@"instrument"] boolValue];
        const uint8_t on[] = {0x90, 60, 100}, off[] = {0x80, 60, 0};
        if (instrument) hd_au_midi(unit, 0, on, 3);
        double energy = 0;
        for (int block = 0; block < 200; ++block) {
            float l[256] = {0}, r[256] = {0};
            if (!instrument) l[0] = r[0] = 0.5;
            hd_au_transport(unit, 120, block * 256.0 * 2 / 48000, 0, block * 256, 4, 4, 1);
            CHECK(hd_au_render(unit, l, r, 256, !instrument), "render failed");
            for (int i = 0; i < 256; ++i) {
                CHECK(isfinite(l[i]) && isfinite(r[i]), "non-finite audio");
                energy += l[i] * l[i] + r[i] * r[i];
            }
        }
        if (instrument) hd_au_midi(unit, 0, off, 3);
        CHECK(energy > 0.00001, "no audible samples from the plugin");
        printf("Audio energy: %.6f\n", energy);
        size_t length = 0;
        uint8_t *state = hd_au_state(unit, &length);
        CHECK(state && length, "state could not be saved");
        CHECK(hd_au_load_state(unit, state, length), "state could not be restored");
        hd_au_free((char *)state);
        printf("State round trip: %zu bytes\n", length);
        if (testEditor) {
            CHECK(hd_au_open_editor(unit), "editor request failed");
            deadline = [NSDate dateWithTimeIntervalSinceNow:30];
            while (hd_au_editor_open(unit) == 1 && deadline.timeIntervalSinceNow > 0) tick();
            CHECK(hd_au_editor_open(unit) == 2, "editor window was not created");
            NSWindow *editor = host.childWindows.firstObject;
            CHECK(editor && editor.parentWindow == host, "editor is not owned by the DAW window");
            [host makeKeyAndOrderFront:nil];
            tick();
            NSArray<NSWindow *> *ordered = app.orderedWindows;
            CHECK([ordered indexOfObject:editor] < [ordered indexOfObject:host], "editor fell behind the DAW after keyboard focus returned");
            CHECK(editor.visible, "editor hidden while using the DAW keyboard");
            printf("Native editor stays above its owner when keyboard focus returns\n");
            deadline = [NSDate dateWithTimeIntervalSinceNow:1];
            while (deadline.timeIntervalSinceNow > 0) tick();
            hd_au_close_editor(unit);
            CHECK(!hd_au_editor_open(unit), "editor did not close");
            CHECK(host.childWindows.count == 0, "closed editor remains attached to the DAW");
            [host close];
        }
        hd_au_reset(unit);
        hd_au_destroy(unit);
        puts("AUv3 installed-plugin test passed");
    }
    return 0;
}
