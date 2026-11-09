<?php

declare(strict_types=1);

namespace Pam\Desktop\System;

use DateTimeZone;

/**
 * PHP's default time zone follows the computer, like every native app.
 *
 * PHP ignores the system zone and falls back to UTC when `date.timezone` is
 * not configured, so `date()` and `new DateTime()` were three hours off in
 * Brazil. `App::run()` applies the system zone unless php.ini (or `-d`)
 * already chose one.
 */
final class SystemTimezone
{
    /** Sets the default zone from the system when php.ini chose none; returns the zone in use. */
    public static function apply(): string
    {
        // get_cfg_var() sees php.ini and -d only: ini_get() also reports
        // PHP's built-in "UTC" default, which is not a choice.
        $configured = get_cfg_var('date.timezone');
        if (!is_string($configured) || $configured === '') {
            $zone = self::detect();
            if ($zone !== null) {
                date_default_timezone_set($zone);
            }
        }

        return date_default_timezone_get();
    }

    /** The computer's IANA zone (TZ, ICU's system zone, /etc/timezone, /etc/localtime), or null. */
    public static function detect(): ?string
    {
        $candidates = [];
        $environment = getenv('TZ');
        if (is_string($environment) && $environment !== '') {
            $candidates[] = ltrim($environment, ':');
        }
        if (class_exists(\IntlTimeZone::class)) {
            $default = \IntlTimeZone::createDefault();
            $candidates[] = $default->getID();
        }
        if (is_file('/etc/timezone')) {
            $candidates[] = trim((string) @file_get_contents('/etc/timezone'));
        }
        $link = @readlink('/etc/localtime');
        if (is_string($link) && preg_match('~zoneinfo/(.+)$~', $link, $match) === 1) {
            $candidates[] = $match[1];
        }
        $known = array_flip(DateTimeZone::listIdentifiers(DateTimeZone::ALL_WITH_BC));
        foreach ($candidates as $candidate) {
            if ($candidate !== '' && isset($known[$candidate])) {
                return $candidate;
            }
        }

        return null;
    }
}
