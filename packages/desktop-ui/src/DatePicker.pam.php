<?php

declare(strict_types=1);
namespace Pam\DesktopUi;
use Closure;
use DateTimeImmutable;
use InvalidArgumentException;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\I18n\FrameworkText;

/** Shared native calendar: 42 bounded day controls, no timers, webviews or native text parsing. */
final class DatePicker extends Component
{
    #[State] public bool $open = false;
    /** The field holds the keyboard focus (it takes it back when the picker applies, like PrimeVue). */
    #[State] public bool $focused = false;
    public readonly string $fieldReference;
    #[State] public string $monthLabel = '';
    /** @var list<CalendarDay> */
    #[State] public array $days = [];
    public readonly DatePickerLocale $locale;
    /** Empty-field text; the localized "Select date" when none is given. */
    public readonly string $placeholder;
    public readonly AutocompleteInputStyle $fieldStyle;
    use Concerns\FollowsSelectTheme;
    public readonly string $host;
    public readonly Select $hours;
    public readonly Select $minutes;
    private DateTimeImmutable $month;
    private ?\Pam\Desktop\Composables\TimezoneState $timezone = null;
    private ?DateTimeImmutable $start = null;
    private ?DateTimeImmutable $end = null;
    /** @param Closure(?DateSelection):void $changed */
    public function __construct(#[State] public ?DateSelection $value, private readonly Closure $changed, public readonly DatePickerMode $mode = DatePickerMode::Date, #[State] public bool $enabled = true, ?string $placeholder = null, ?DatePickerLocale $locale = null, SelectTheme|Closure|null $theme = null, public readonly int $minimumYear = 1900, public readonly int $maximumYear = 2100, public readonly DatePickerPresentation $presentation = DatePickerPresentation::Button, public readonly string $accessibilityLabel = '', public readonly ?string $displayFormat = null, ?AutocompleteInputStyle $fieldStyle = null, public readonly string $rangeSeparator = ' — ', public readonly bool $showFieldIcon = true)
    {
        if ($minimumYear < 1 || $maximumYear > 9999 || $minimumYear > $maximumYear) throw new InvalidArgumentException('Invalid calendar year bounds.');
        if (mb_strlen($rangeSeparator) > 16) throw new InvalidArgumentException('Range separators are at most 16 characters.');
        $this->placeholder=$placeholder??FrameworkText::get('datePicker.placeholder');$this->locale=$locale??DatePickerLocale::system();$this->fieldStyle=$fieldStyle??new AutocompleteInputStyle(40,14,400,12,8);$this->followTheme($theme);$this->host='date-picker-'.spl_object_id($this);$this->fieldReference=$this->host.'-field';
        $this->month=($value->start??new DateTimeImmutable())->modify('first day of this month')->setTime(0,0);
        $this->hours=new Select(array_map(static fn(int $v):SelectOption=>new SelectOption($v,str_pad((string)$v,2,'0',STR_PAD_LEFT)),range(0,23)),0,$this->hourChanged(...),host:$this->host,theme:$this->themeSource,label:$this->locale->hour);
        $this->minutes=new Select(array_map(static fn(int $v):SelectOption=>new SelectOption($v,str_pad((string)$v,2,'0',STR_PAD_LEFT)),range(0,59)),0,$this->minuteChanged(...),host:$this->host,theme:$this->themeSource,label:$this->locale->minute);
    }
    public function setup():void{$this->timezone=\Pam\Desktop\useTimezone();}
    private function local(DateTimeImmutable $date):DateTimeImmutable{return $this->mode===DatePickerMode::DateTime&&$this->timezone!==null?$date->setTimezone($this->timezone->zone()):$date;}
    public function label():string
    {
        if($this->value===null)return $this->placeholder;
        $format=($this->displayFormat??$this->locale->dateFormat).($this->mode===DatePickerMode::DateTime?' H:i':'');
        return $this->local($this->value->start)->format($format).($this->value->end===null?'':$this->rangeSeparator.$this->local($this->value->end)->format($format));
    }
    public function field(): bool { return $this->presentation === DatePickerPresentation::Field; }
    /**
     * The field's label as Chromium lays out an `<input type="date">`: every
     * date part (dd, mm, aaaa, 07) is an edit field with 1px of horizontal
     * padding, the separators are plain text. A range keeps one run.
     *
     * @return list<array{text: string, padding: int}>
     */
    public function fieldSegments(): array
    {
        $label = $this->label();
        if ($this->mode === DatePickerMode::Range || preg_match_all('/[\p{L}\p{N}]+|[^\p{L}\p{N}]+/u', $label, $parts) === false) {
            return [['text' => $label, 'padding' => 0]];
        }
        return array_map(static fn (string $part): array => ['text' => $part, 'padding' => preg_match('/^[\p{L}\p{N}]/u', $part) === 1 ? 1 : 0], $parts[0]);
    }
    public function triggerKey(string $payload): void
    {
        $event=\Pam\Desktop\KeyEvent::fromPayload($payload);
        if (!$event->repeat && !$event->control && !$event->alt && !$event->meta && in_array($event->key,[' ','Enter'],true)) $this->show();
    }
    public function show():void
    {
        if(!$this->enabled)return;
        $this->start=$this->value===null?null:$this->local($this->value->start);$this->end=$this->value?->end;
        $base=$this->start??($this->timezone?->now()??new DateTimeImmutable());
        $year=max($this->minimumYear,min($this->maximumYear,(int)$base->format('Y')));
        $this->month=$base->setDate($year,(int)$base->format('n'),1)->setTime(0,0);
        $this->hours->value=(int)$base->format('H');$this->minutes->value=(int)$base->format('i');
        $this->rebuild();$this->open=true;
    }
    public function dismiss():void{$this->open=false;}
    public function previous():void{$this->move(-1);}
    public function next():void{$this->move(1);}
    private function move(int $delta):void
    {
        $next=$this->month->modify(($delta>0?'+':'').$delta.' month');$year=(int)$next->format('Y');
        if($year<$this->minimumYear||$year>$this->maximumYear)return;$this->month=$next;$this->rebuild();
    }
    public function select(string $date):void
    {
        if(!$this->open||!$this->enabled)return;
        $day=null;foreach($this->days as $candidate)if($candidate->date===$date&&$candidate->enabled)$day=$candidate;
        if($day===null)return;
        $selected=DateTimeImmutable::createFromFormat('!Y-m-d',$date,$this->month->getTimezone());
        if($selected===false)return;
        if($this->mode===DatePickerMode::Range){if($this->start===null||$this->end!==null){$this->start=$selected;$this->end=null;}elseif($selected<$this->start){$this->end=$this->start;$this->start=$selected;}else $this->end=$selected;}
        else{$this->start=$selected;$this->end=null;$this->applyTime();}
        $this->rebuild();
    }
    private function hourChanged(int|string|null $value):void{if(is_int($value)){$this->hours->value=$value;$this->applyTime();}}
    private function minuteChanged(int|string|null $value):void{if(is_int($value)){$this->minutes->value=$value;$this->applyTime();}}
    private function applyTime():void{if($this->mode===DatePickerMode::DateTime&&$this->start!==null&&is_int($this->hours->value)&&is_int($this->minutes->value))$this->start=$this->start->setTime($this->hours->value,$this->minutes->value);}
    public function hasTime():bool{return $this->mode===DatePickerMode::DateTime;}
    public function complete():bool{return $this->start!==null&&($this->mode!==DatePickerMode::Range||$this->end!==null);}
    public function apply():void{if(!$this->enabled||!$this->open||!$this->complete()||$this->start===null)return;$this->value=new DateSelection($this->start,$this->end);$this->open=false;if($this->field()){$this->focused=true;\Pam\Desktop\System\ViewRef::of($this->fieldReference)->focus();}($this->changed)($this->value);}
    public function fieldFocus():void{$this->focused=true;}
    public function fieldBlur():void{$this->focused=false;}
    public function fieldBorder():int{return $this->focused&&$this->theme->focusBorder!==null?$this->theme->focusBorder:$this->theme->border;}
    public function fieldRing():int{return $this->focused&&$this->theme->focusRing!==null?$this->theme->focusRing:0;}
    public function clear():void{if(!$this->enabled)return;$this->value=null;$this->open=false;($this->changed)(null);}
    private function rebuild():void
    {
        $this->monthLabel=$this->locale->months[(int)$this->month->format('n')-1].' '.$this->month->format('Y');
        $first=$this->month->modify('-'.$this->month->format('w').' days');$days=[];
        for($i=0;$i<42;++$i){$date=$first->modify('+'.$i.' days');$key=$date->format('Y-m-d');$selected=$this->start!==null&&($key===$this->start->format('Y-m-d')||($this->end!==null&&$key>=$this->start->format('Y-m-d')&&$key<=$this->end->format('Y-m-d')));$year=(int)$date->format('Y');$days[]=new CalendarDay($key,$date->format('j'),$date->format('m')===$this->month->format('m'),$selected,$year>=$this->minimumYear&&$year<=$this->maximumYear);}
        $this->days=$days;
    }
}
?>
<template><Column><Button p-if="!field()" :label="label()" on:press="show" :enabled="$enabled" height="40" padding="8 12" :backgroundColor="$theme->surface" :textColor="$theme->text" :borderColor="$theme->border" borderWidth="1" borderRadius="8" /><Pressable class="date-field" p-if="field()" :nativeRef="$fieldReference" on:press="show" on:keyDown="triggerKey" on:focus="fieldFocus" on:blur="fieldBlur" :enabled="$enabled" :opacity="$enabled ? 1 : 0.5" :accessibilityLabel="$accessibilityLabel !== '' ? $accessibilityLabel : $placeholder" testId="date-picker-field" :height="$fieldStyle->height" :paddingLeft="$fieldStyle->paddingHorizontal" :paddingRight="$fieldStyle->paddingHorizontal" :backgroundColor="$theme->surface" :borderColor="fieldBorder()" :shadowColor="fieldRing()" :shadowSpreadRadius="fieldRing() === 0 ? 0 : $theme->focusRingWidth" borderWidth="1" :borderRadius="$fieldStyle->radius"><Row width="100%" alignItems="center" gap="8"><Row flexGrow="1" flexShrink="1" overflow="hidden"><Text p-for="$segment in fieldSegments()" numberOfLines="1" :paddingLeft="$segment['padding']" :paddingRight="$segment['padding']" :textColor="$theme->text" :fontSize="$fieldStyle->fontSize" :fontWeight="$fieldStyle->fontWeight" :lineHeight="$fieldStyle->lineHeight ?? 0">{{ $segment['text'] }}</Text></Row><Icon p-if="$showFieldIcon && $theme->iconFont !== null" name="calendar" :font="$theme->iconFont" size="14" :color="$theme->text" /><Column p-if="$showFieldIcon && $theme->iconFont === null" width="16" height="16"><Row height="2.1" paddingTop="0.7" paddingLeft="2" gap="6.2"><View width="1.4" height="1.4" :backgroundColor="$theme->triggerIconColor ?? $theme->text"/><View width="1.4" height="1.4" :backgroundColor="$theme->triggerIconColor ?? $theme->text"/></Row><Column marginLeft="0.25" width="12.5" height="12.6" borderWidth="1.2" :borderColor="$theme->triggerIconColor ?? $theme->text" borderRadius="1"><View height="2" :backgroundColor="$theme->triggerIconColor ?? $theme->text"/></Column></Column></Row></Pressable><Modal p-if="$open" visible="true" presentation="dialog" on:requestClose="dismiss"><SelectProvider :host="$host"><Column width="100%" height="100%" alignItems="center" justifyContent="center"><Column width="360" padding="20" gap="14" :backgroundColor="$theme->surface" borderRadius="12" borderWidth="1" :borderColor="$theme->border"><Row alignItems="center"><Button label="‹" on:press="previous" width="40" height="36" :textColor="$theme->text" /><Text flexGrow="1" textAlign="center" fontWeight="700" :textColor="$theme->text">{{ $monthLabel }}</Text><Button label="›" on:press="next" width="40" height="36" :textColor="$theme->text" /></Row><Grid columns="7" gridColumnGap="2" gridRowGap="4"><Text p-for="$weekday in $locale->weekdays" p-key="$weekday" span="1" textAlign="center" fontSize="11" :textColor="$theme->muted">{{ $weekday }}</Text><Button p-for="$day in $days" p-key="$day->date" span="1" height="34" :label="$day->label" on:press="select($day->date)" :enabled="$day->enabled" :backgroundColor="$day->selected ? $theme->highlight : $theme->surface" :textColor="$day->currentMonth ? $theme->text : $theme->muted" borderRadius="6" /></Grid><Row p-if="hasTime()" gap="12"><Column flexGrow="1" gap="6"><Text :textColor="$theme->muted">{{ $locale->hour }}</Text><Slot :content="$hours" /></Column><Column flexGrow="1" gap="6"><Text :textColor="$theme->muted">{{ $locale->minute }}</Text><Slot :content="$minutes" /></Column></Row><Row gap="8" justifyContent="end"><Button :label="$locale->clear" on:press="clear" :textColor="$theme->muted" height="36" /><Button :label="$locale->cancel" on:press="dismiss" :textColor="$theme->text" height="36" /><Button :label="$locale->apply" on:press="apply" :enabled="complete()" :backgroundColor="$theme->highlight" :textColor="$theme->text" height="36" padding="0 12" borderRadius="6" /></Row></Column></Column></SelectProvider></Modal></Column></template>

<style scoped>
.date-field { justify-content: center; padding: 8px 12px; }
.date-field:focus { border-color: #60A5FA; }
</style>
