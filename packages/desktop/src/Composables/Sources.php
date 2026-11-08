<?php

declare(strict_types=1);

namespace Pam\Desktop\Composables;

use Pam\Desktop\App;
use Pam\Desktop\Appearance;
use Pam\Desktop\AppearanceMode;
use Pam\Desktop\AppState;
use Pam\Desktop\DeviceStatus;
use Pam\Desktop\I18n\I18n;
use Pam\Desktop\System\DeviceInfo;
use Pam\Desktop\Internal\ComponentLifecycle;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\KeyboardMetrics;
use Pam\Desktop\System\DeviceStatus as DeviceStatusModule;
use Pam\Desktop\System\Keyboard;
use Pam\Desktop\UserInterfaceAppearance;
use Pam\Desktop\WindowMetrics;

/**
 * The shared push sources behind the framework composables.
 *
 * @internal
 */
final class Sources
{
    private function __construct()
    {
    }

    /** @return SharedSource<array{0: UserInterfaceAppearance, 1: AppearanceMode}> */
    public static function appearance(): SharedSource
    {
        return SharedSource::named(
            'appearance',
            static fn (): array => [Appearance::current(), Appearance::mode()],
            static function (\Closure $push): \Closure {
                $id = Appearance::onChange(static function (UserInterfaceAppearance $appearance, AppearanceMode $mode) use ($push): void {
                    $push([$appearance, $mode]);
                });

                return static fn () => Appearance::unsubscribe($id);
            },
        );
    }

    /** @return SharedSource<WindowMetrics> */
    public static function window(): SharedSource
    {
        return SharedSource::named(
            'window',
            static fn (): WindowMetrics => Runtime::windowMetrics(),
            static function (\Closure $push): \Closure {
                $id = Runtime::onWindowMetrics($push);

                return static fn () => Runtime::offWindowMetrics($id);
            },
        );
    }

    /** @return SharedSource<KeyboardMetrics> */
    public static function keyboard(): SharedSource
    {
        return SharedSource::named(
            'keyboard',
            static fn (): KeyboardMetrics => Keyboard::metrics(),
            static function (\Closure $push): \Closure {
                $id = Keyboard::onChange($push);

                return static fn () => Keyboard::offChange($id);
            },
        );
    }

    /** @return SharedSource<AppState> */
    public static function appState(): SharedSource
    {
        return SharedSource::named(
            'appState',
            static fn (): AppState => App::state() ?? ComponentLifecycle::currentAppState(),
            static function (\Closure $push): \Closure {
                $id = App::onStateChange($push);

                return static fn () => App::offStateChange($id);
            },
        );
    }

    /** @var DeviceStatus|null last status the host reported, kept across restarts of the watch */
    private static ?DeviceStatus $lastStatus = null;

    /** @return SharedSource<DeviceStatus|null> */
    public static function network(): SharedSource
    {
        return SharedSource::named(
            'network',
            static fn (): ?DeviceStatus => self::$lastStatus,
            static function (\Closure $push): \Closure {
                // DeviceStatus samples network/power state on the host interval.
                // Keep that subscription only while the app is visible.
                $watch = null;
                $start = static function () use (&$watch, $push): void {
                    if ($watch !== null) {
                        return;
                    }
                    $watch = DeviceStatusModule::watch(static function (DeviceStatus $status) use ($push): void {
                        self::$lastStatus = $status;
                        $push($status);
                    }, 2_000);
                };
                $stop = static function () use (&$watch): void {
                    if ($watch === null) {
                        return;
                    }
                    DeviceStatusModule::unwatch($watch);
                    $watch = null;
                };
                $state = App::state() ?? ComponentLifecycle::currentAppState();
                if ($state !== AppState::Background) {
                    $start();
                }
                $stateSubscription = App::onStateChange(static function (AppState $state) use ($start, $stop): void {
                    if ($state === AppState::Background) {
                        $stop();
                    } else {
                        $start();
                    }
                });

                return static function () use ($stop, $stateSubscription): void {
                    $stop();
                    App::offStateChange($stateSubscription);
                };
            },
        );
    }

    /** @var DeviceInfo|null last system info the host reported (locale, time zone) */
    private static ?DeviceInfo $lastDevice = null;

    /** Asks the host for DeviceInfo while the device source runs; null when stopped. */
    private static ?\Closure $askDevice = null;
    private static bool $deviceAsked = false;

    /**
     * Device locale and time zone. Lazy: nothing is asked until a reader
     * first reads a value (requestDevice()); then once more each time the
     * app returns to the foreground (the user may have changed them in the
     * system settings). Never polled.
     *
     * @return SharedSource<DeviceInfo|null>
     */
    public static function device(): SharedSource
    {
        return SharedSource::named(
            'device',
            static fn (): ?DeviceInfo => self::$lastDevice,
            static function (\Closure $push): \Closure {
                $running = true;
                $ask = static function () use (&$running, $push): void {
                    DeviceInfo::get(static function (DeviceInfo $info) use (&$running, $push): void {
                        if (!$running) return;
                        self::$lastDevice = $info;
                        $push($info);
                    }, static function (): void {
                        // Keep the last known values; the next foreground asks again.
                    });
                };
                self::$askDevice = $ask;
                self::$deviceAsked = false;
                $id = App::onStateChange(static function (AppState $state) use ($ask): void {
                    if ($state === AppState::Active && self::$deviceAsked) $ask();
                });

                return static function () use (&$running, $id): void {
                    $running = false;
                    self::$askDevice = null;
                    self::$deviceAsked = false;
                    App::offStateChange($id);
                };
            },
        );
    }

    /** @internal Called by useLocale()/useTimezone() reads: the first one asks the host. */
    public static function requestDevice(): void
    {
        if (self::$deviceAsked || self::$askDevice === null) return;
        self::$deviceAsked = true;
        (self::$askDevice)();
    }

    /** @internal The last system info the host reported (FrameworkText reads its locale), null before any. */
    public static function lastDevice(): ?DeviceInfo
    {
        return self::$lastDevice;
    }

    /** @internal Tests: the system locale and time zone as if the host reported them. */
    public static function reportDevice(DeviceInfo $info): void
    {
        self::$lastDevice = $info;
        self::device()->signal->set($info);
    }

    /**
     * Locale and time zone overrides (I18n::use(), I18n::useTimezone()).
     *
     * @return SharedSource<array{0: ?string, 1: ?string}>
     */
    public static function i18n(): SharedSource
    {
        return SharedSource::named(
            'i18n',
            static fn (): array => [I18n::preferredLocale(), I18n::timezone()],
            static function (\Closure $push): \Closure {
                $id = I18n::onChange(static fn () => $push([I18n::preferredLocale(), I18n::timezone()]));

                return static fn () => I18n::offChange($id);
            },
        );
    }

    /** @internal */
    public static function reset(): void
    {
        self::$lastDevice = null;
        self::$askDevice = null;
        self::$deviceAsked = false;
        self::$lastStatus = null;
        SharedSource::reset();
    }
}
