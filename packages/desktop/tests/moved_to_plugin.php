<?php

declare(strict_types=1);

use Pam\Desktop\Plugin\MovedToPlugin;
use Pam\Desktop\Plugin\PluginException;

// 1.46: features moved out of the core keep a deprecated alias for one minor.
// Without the plugin, the old name fails with the exact composer require.
foreach ([
    'Pam\\Desktop\\System\\Location' => 'pushinbr/pam-desktop-location',
    'Pam\\Desktop\\LocationPosition' => 'pushinbr/pam-desktop-location',
    'Pam\\Desktop\\System\\Contacts' => 'pushinbr/pam-desktop-contacts',
    'Pam\\Desktop\\System\\Sms' => 'pushinbr/pam-desktop-contacts',
    'Pam\\Desktop\\System\\Sensors' => 'pushinbr/pam-desktop-sensors',
    'Pam\\Desktop\\SensorType' => 'pushinbr/pam-desktop-sensors',
    'Pam\\Desktop\\System\\AudioRecorder' => 'pushinbr/pam-desktop-audio',
    'Pam\\Desktop\\System\\ImageEditor' => 'pushinbr/pam-desktop-media-editor',
    'Pam\\Desktop\\ImageCropRatio' => 'pushinbr/pam-desktop-media-editor',
] as $legacy => $package) {
    $message = '';
    try {
        class_exists($legacy);
    } catch (PluginException $error) {
        $message = $error->getMessage();
    }
    $assert(
        str_contains($message, $legacy.' moved out of the PAM Desktop core into '.$package)
            && str_contains($message, 'composer require '.$package),
        "{$legacy} must point to {$package} when the plugin is missing.",
    );
}

foreach ([
    'Pam\\Desktop\\System\\Toast' => 'pushinbr/pam-desktop-toast',
    'Pam\\Desktop\\System\\ToastType' => 'pushinbr/pam-desktop-toast',
    'Pam\\Desktop\\System\\ToastPosition' => 'pushinbr/pam-desktop-toast',
    'Pam\\Desktop\\UI\\DrawerLayoutAndroid' => 'pushinbr/pam-desktop-drawer',
    'Pam\\Desktop\\Navigation\\DrawerNavigator' => 'pushinbr/pam-desktop-drawer',
    'Pam\\Desktop\\Navigation\\DrawerRouter' => 'pushinbr/pam-desktop-drawer',
    'Pam\\Desktop\\Navigation\\NavigationDrawerItem' => 'pushinbr/pam-desktop-drawer',
    'Pam\\Desktop\\Navigation\\DrawerType' => 'pushinbr/pam-desktop-drawer',
    'Pam\\Desktop\\Navigation\\DrawerPosition' => 'pushinbr/pam-desktop-drawer',
    'Pam\\Desktop\\Navigation\\DrawerBackBehavior' => 'pushinbr/pam-desktop-drawer',
    'Pam\\Desktop\\Navigation\\DrawerKeyboardDismissMode' => 'pushinbr/pam-desktop-drawer',
    'Pam\\Desktop\\Navigation\\DrawerStatusBarAnimation' => 'pushinbr/pam-desktop-drawer',
    'Pam\\Desktop\\Routing\\DrawerRegistrar' => 'pushinbr/pam-desktop-drawer',
    'Pam\\Desktop\\System\\Notifications' => 'pushinbr/pam-desktop-notifications',
    'Pam\\Desktop\\System\\MediaLibrary' => 'pushinbr/pam-desktop-media',
    'Pam\\Desktop\\System\\IncomingShares' => 'pushinbr/pam-desktop-share-extension',
    'Pam\\Desktop\\System\\Biometrics' => 'pushinbr/pam-desktop-auth',
    'Pam\\Desktop\\UI\\WebView' => 'pushinbr/pam-desktop-webview',
] as $legacy => $package) {
    if ($testPluginRoots !== []) continue;
    $message = '';
    try { class_exists($legacy); } catch (PluginException $error) { $message = $error->getMessage(); }
    $assert(str_contains($message, 'composer require '.$package), $legacy.' must name its plugin.');
}

// With the plugin installed the old name is the plugin class itself.
eval('namespace Pam\\Desktop\\MovedFixture; enum Ratio: int { case Square = 1; }');
MovedToPlugin::alias('Pam\\Desktop\\LegacyRatio', 'Pam\\Desktop\\MovedFixture\\Ratio', 'pushinbr/pam-desktop-fixture');
$assert(
    \Pam\Desktop\LegacyRatio::Square === \Pam\Desktop\MovedFixture\Ratio::Square
        && MovedToPlugin::aliased()['Pam\\Desktop\\LegacyRatio'] === 'pushinbr/pam-desktop-fixture',
    'A moved class must alias the plugin class so instanceof and enum cases keep working.',
);

echo "MOVED_TO_PLUGIN_OK aliases=".count(MovedToPlugin::aliased())."\n";
