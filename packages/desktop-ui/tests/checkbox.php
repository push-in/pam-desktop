<?php

declare(strict_types=1);
use Pam\Desktop\App;
use Pam\Desktop\Internal\Wire;
use Pam\DesktopUi\Checkbox;
$loader=require dirname(__DIR__,3).'/vendor/autoload.php';$loader->addPsr4('Pam\\DesktopUi\\',dirname(__DIR__).'/src');App::components(dirname(__DIR__).'/src',sys_get_temp_dir().'/pam-desktop-checkbox-tests');
$values=[];$checkbox=new Checkbox(false,static function(bool $value)use(&$values):void{$values[]=$value;},label:'Enabled');$mounted=Pam\Desktop\Testing\mount($checkbox);
try{
$checkbox->toggle();if($values!==[true]||$checkbox->checked)throw new RuntimeException('Checkbox must remain controlled.');$checkbox->checked=true;$checkbox->key(Wire::map(['key'=>' ']));if($values!==[true,false])throw new RuntimeException('Space payload must toggle once.');$checkbox->checked=false;$checkbox->key(new Pam\Desktop\KeyEvent('Enter'));if($values!==[true,false,true])throw new RuntimeException('Native KeyEvent must toggle once.');$checkbox->key(Wire::map(['key'=>'Enter','repeat'=>true]));if(count($values)!==3)throw new RuntimeException('Repeat must not toggle.');$checkbox->enabled=false;$checkbox->key(new Pam\Desktop\KeyEvent('Enter'));$checkbox->toggle();if(count($values)!==3)throw new RuntimeException('Disabled checkbox must not change.');$checkbox->checked=true;$mounted->render();if($checkbox->checkedState()!==Pam\Desktop\AccessibilityCheckedState::Checked->value)throw new RuntimeException('Accessibility state incorrect.');echo "Native controlled checkbox, wire/native keyboard events, disabled state and accessibility: passed\n";
}finally{$mounted->unmount();}
