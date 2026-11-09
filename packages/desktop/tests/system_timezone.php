<?php

declare(strict_types=1);

require dirname(__DIR__, 3).'/vendor/autoload.php';

use Pam\Desktop\System\SystemTimezone;

function timezoneCheck(bool $ok, string $message): void { if (!$ok) throw new RuntimeException($message); }

putenv('TZ=America/Sao_Paulo');
timezoneCheck(SystemTimezone::detect() === 'America/Sao_Paulo', 'TZ names the system zone.');
putenv('TZ=:Europe/Lisbon');
timezoneCheck(SystemTimezone::detect() === 'Europe/Lisbon', 'A leading colon in TZ is accepted.');
putenv('TZ=Not/AZone');
timezoneCheck(SystemTimezone::detect() !== 'Not/AZone', 'Unknown zones are ignored.');
putenv('TZ=America/Sao_Paulo');
if (get_cfg_var('date.timezone') === false) {
    timezoneCheck(SystemTimezone::apply() === 'America/Sao_Paulo' && date_default_timezone_get() === 'America/Sao_Paulo', 'Without date.timezone the system zone becomes the default.');
} else {
    $configured = date_default_timezone_get();
    timezoneCheck(SystemTimezone::apply() === $configured, 'A configured date.timezone is kept.');
}
echo "System time zone default passed.\n";
