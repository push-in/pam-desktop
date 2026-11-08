<?php

declare(strict_types=1);
namespace Pam\DesktopUi;
use InvalidArgumentException;
use Pam\Desktop\I18n\FrameworkText;
use Pam\Desktop\I18n\I18n;
final readonly class DatePickerLocale
{
    /**
     * @param list<string> $months
     * @param list<string> $weekdays
     */
    public function __construct(public array $months = ['January','February','March','April','May','June','July','August','September','October','November','December'], public array $weekdays = ['Su','Mo','Tu','We','Th','Fr','Sa'], public string $apply = 'Apply', public string $cancel = 'Cancel', public string $clear = 'Clear', public string $hour = 'Hour', public string $minute = 'Minute', public string $dateFormat = 'm/d/Y', public string $datePlaceholder = 'mm/dd/yyyy')
    {
        if (count($months)!==12 || count($weekdays)!==7) throw new InvalidArgumentException('A calendar locale requires 12 months and 7 weekdays.');
    }
    public static function portuguese(): self { return self::forLocale('pt-BR'); }
    public static function spanish(): self { return self::forLocale('es'); }
    public static function english(): self { return new self(); }

    /** The calendar copy of a locale tag from the framework catalog (FrameworkText); unknown languages get English. */
    public static function forLocale(string $locale): self
    {
        $text = static fn (string $key): string => FrameworkText::get('calendar.'.$key, [], $locale);

        return new self(FrameworkText::list('calendar.months', $locale), FrameworkText::list('calendar.weekdays', $locale), $text('apply'), $text('cancel'), $text('clear'), $text('hour'), $text('minute'), $text('dateFormat'), $text('datePlaceholder'));
    }

    /**
     * The calendar of the app language (I18n::use()), else of the process
     * locale (LC_ALL, LC_TIME, LANG), like a browser date input; else the
     * framework locale. The default of DatePicker.
     */
    public static function system(): self
    {
        $preferred = I18n::preferredLocale();
        if ($preferred !== null) return self::forLocale($preferred);
        foreach (['LC_ALL', 'LC_TIME', 'LANG'] as $name) {
            $value = getenv($name);
            if (is_string($value) && $value !== '' && $value !== 'C' && $value !== 'POSIX' && !str_starts_with($value, 'C.')) {
                return self::forLocale($value);
            }
        }
        return self::forLocale(FrameworkText::locale());
    }
}
