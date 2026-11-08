<?php

declare(strict_types=1);
use Pam\Desktop\App;
use Pam\DesktopUi\{DatePicker,DatePickerMode,DatePickerLocale,DateSelection};
$loader=require dirname(__DIR__,3).'/vendor/autoload.php';$loader->addPsr4('Pam\\DesktopUi\\',dirname(__DIR__).'/src');App::components(dirname(__DIR__).'/src',sys_get_temp_dir().'/pam-desktop-calendar-tests');
function calendarCheck(bool $ok,string $message):void{if(!$ok)throw new RuntimeException($message);}
$values=[];$calendar=new DatePicker(new DateSelection(new DateTimeImmutable('2024-02-10')),static function(?DateSelection $v)use(&$values):void{$values[]=$v;},DatePickerMode::Range,locale:DatePickerLocale::portuguese());$mounted=Pam\Desktop\Testing\mount($calendar);
try{
$calendar->show();$mounted->render();calendarCheck(count($calendar->days)===42,'Calendar renders bounded six weeks.');$calendar->select('2024-02-29');$calendar->apply();calendarCheck(count($values)===1&&$values[0]->end->format('Y-m-d')==='2024-02-29','Leap-day range retains local dates.');
$calendar->show();$calendar->select('2024-02-20');$calendar->select('2024-02-04');$calendar->apply();calendarCheck($values[1]->start->format('Y-m-d')==='2024-02-04'&&$values[1]->end->format('Y-m-d')==='2024-02-20','Reverse selection normalizes range.');
$calendar->show();$calendar->select('2024-02-31');$calendar->dismiss();calendarCheck(count($values)===2,'Invalid dates and dismissed changes emit nothing.');
$calendar->enabled=false;$calendar->clear();calendarCheck(count($values)===2,'Disabled control cannot mutate.');
$time=new DatePicker(new DateSelection(new DateTimeImmutable('2024-12-31T23:45:00-03:00')),static function(?DateSelection $v)use(&$values):void{$values[]=$v;},DatePickerMode::DateTime);$time->show();$time->select('2024-12-30');$time->apply();calendarCheck($values[2]->start->format('Y-m-d H:i P')==='2024-12-30 23:45 -03:00','Datetime preserves local timezone and selected time.');
echo "Native DatePicker tags, leap dates, reversed range, cancellation, disabled state and timezone: passed\n";
}finally{$mounted->unmount();}
$field=new DatePicker(null,static function(?DateSelection $value):void{},placeholder:'mm/dd/yyyy',locale:DatePickerLocale::portuguese(),presentation:Pam\DesktopUi\DatePickerPresentation::Field,accessibilityLabel:'Data inicial',displayFormat:'m/d/Y');
$mounted=Pam\Desktop\Testing\mount($field);
try {
 calendarCheck($field->field()&&str_contains($mounted->text(),'mm/dd/yyyy'),'Field trigger renders an explicit empty date mask.');
 $field->triggerKey(Pam\Desktop\Internal\Wire::map(['key'=>'Enter']));$mounted->render();calendarCheck($field->open&&count($field->days)===42,'Field keyboard activation opens the same bounded calendar.');
 calendarCheck($field->locale->apply==='Aplicar'&&$field->locale->cancel==='Cancelar'&&$field->locale->dateFormat==='d/m/Y','Field appearance preserves Portuguese popup labels.');
 $field->dismiss();$field->enabled=false;$field->triggerKey(Pam\Desktop\Internal\Wire::map(['key'=>'Enter']));calendarCheck(!$field->open,'Disabled field cannot open from keyboard.');
 $field->value=new DateSelection(new DateTimeImmutable('2024-12-31'));calendarCheck($field->label()==='12/31/2024','Explicit field format agrees with the empty mask.');
 $default=new DatePicker(null,static function(?DateSelection $value):void{});calendarCheck(!$default->field()&&$default->label()===\Pam\Desktop\I18n\FrameworkText::get('datePicker.placeholder'),'Existing consumers keep default button presentation.');
} finally {$mounted->unmount();}
echo "DatePicker field presentation, keyboard, disabled, localization and compatibility passed\n";
$previous=[getenv('LC_ALL'),getenv('LC_TIME'),getenv('LANG')];
putenv('LC_ALL=');putenv('LC_TIME=');putenv('LANG=pt_BR.UTF-8');
$system=DatePickerLocale::system();
if($system->dateFormat!=='d/m/Y'||$system->datePlaceholder!=='dd/mm/aaaa'||$system->months[0]!=='Janeiro')throw new RuntimeException('Portuguese system locale must give the dd/mm/aaaa calendar.');
putenv('LANG=en_US.UTF-8');
if(DatePickerLocale::system()->datePlaceholder!=='mm/dd/yyyy')throw new RuntimeException('Other locales keep the English calendar.');
putenv('LANG=C');putenv('LC_ALL=pt_PT.UTF-8');
if(DatePickerLocale::system()->dateFormat!=='d/m/Y')throw new RuntimeException('LC_ALL takes precedence.');
foreach(['LC_ALL','LC_TIME','LANG'] as $i=>$name){putenv($previous[$i]===false?$name:$name.'='.$previous[$i]);}
echo "DatePickerLocale follows the system locale like a browser date input.\n";
$range=new DatePicker(new DateSelection(new DateTimeImmutable('2026-11-01'),new DateTimeImmutable('2026-11-07')),static function(?DateSelection $v):void{},DatePickerMode::Range,locale:DatePickerLocale::portuguese(),rangeSeparator:' - ',showFieldIcon:false,presentation:Pam\DesktopUi\DatePickerPresentation::Field);
if($range->label()!=='01/11/2026 - 07/11/2026'||$range->showFieldIcon)throw new RuntimeException('Range separator and field icon must be configurable.');
$default=new DatePicker(new DateSelection(new DateTimeImmutable('2026-11-01'),new DateTimeImmutable('2026-11-07')),static function(?DateSelection $v):void{},DatePickerMode::Range,locale:DatePickerLocale::portuguese());
if($default->label()!=='01/11/2026 — 07/11/2026'||!$default->showFieldIcon)throw new RuntimeException('Default range separator and icon stay unchanged.');
echo "DatePicker range separator and optional field icon passed.\n";
