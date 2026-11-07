// Simulator UI regression test for the host-owned AUv3 return toolbar.
// Build the bridge in this test translation unit to exercise the real container.
#import "../src/plugins/auv3/bridge.m"
#import <assert.h>

@interface HDTestPluginController : UIViewController
@end
@implementation HDTestPluginController
- (void)loadView {
    self.view = [UIView new];
    self.view.backgroundColor = UIColor.systemBlueColor;
    // A plugin owns its own navigation items, not the host's toolbar.
    self.navigationItem.leftBarButtonItem = nil;
}
@end
@interface HDEditorTestSceneDelegate : UIResponder <UIWindowSceneDelegate>
@property(nonatomic, strong) UIWindow *window;
@end
@implementation HDEditorTestSceneDelegate
- (void)scene:(UIScene *)scene willConnectToSession:(UISceneSession *)session options:(UISceneConnectionOptions *)options {
    (void)session; (void)options;
    self.window = [[UIWindow alloc] initWithWindowScene:(UIWindowScene *)scene];
    UIViewController *root = [UIViewController new];
    root.view.backgroundColor = UIColor.systemBackgroundColor;
    self.window.rootViewController = root;
    [self.window makeKeyAndVisible];
    dispatch_async(dispatch_get_main_queue(), ^{
        assert(presenter() == root); // Includes winit's legacy application lifecycle.
        HDPluginEditorController *container = [HDPluginEditorController new];
        container.pluginController = [HDTestPluginController new];
        container.pluginTitle = @"Test AUv3 Synth";
        container.modalPresentationStyle = UIModalPresentationPageSheet;
        [root presentViewController:container animated:NO completion:^{
            [container.view layoutIfNeeded];
            UIBarButtonItem *back = container.hostNavigationBar.topItem.leftBarButtonItem;
            assert([back.title isEqualToString:@"Back to Honeydew"]);
            assert(back.enabled && back.target == container);
            assert([back.accessibilityIdentifier isEqualToString:@"honeydew.auv3.back"]);
            for (NSValue *size in @[[NSValue valueWithCGSize:CGSizeMake(600, 900)],
                                    [NSValue valueWithCGSize:CGSizeMake(1024, 768)],
                                    [NSValue valueWithCGSize:CGSizeMake(1366, 1024)]]) {
                container.view.frame = (CGRect){CGPointZero, size.CGSizeValue};
                [container.view setNeedsLayout];
                [container.view layoutIfNeeded];
                CGRect toolbar = container.hostNavigationBar.frame;
                CGRect plugin = container.pluginController.view.frame;
                assert(toolbar.size.height >= 44);
                assert(CGRectGetMinY(plugin) >= CGRectGetMaxY(toolbar) - 0.5);
                assert(plugin.size.width > 0 && plugin.size.height > 0);
            }
            // Exercise the same target/action sent by a real tap.
            [UIApplication.sharedApplication sendAction:back.action to:back.target from:back forEvent:nil];
            dispatch_after(dispatch_time(DISPATCH_TIME_NOW, NSEC_PER_SEC), dispatch_get_main_queue(), ^{
                assert(root.presentedViewController == nil);
                assert(self.window.rootViewController == root);
                puts("AUv3 iPad return toolbar/layout test passed"); fflush(stdout);
                exit(0);
            });
        }];
    });
}
@end
@interface HDEditorTestDelegate : UIResponder <UIApplicationDelegate>
@end
@implementation HDEditorTestDelegate
- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)options {
    (void)application; (void)options;
    return YES;
}
@end
int main(int argc, char **argv) {
    @autoreleasepool { return UIApplicationMain(argc, argv, nil, NSStringFromClass(HDEditorTestDelegate.class)); }
}
