<?php
declare(strict_types=1);
require dirname(__DIR__,3).'/vendor/autoload.php';
use Pam\Desktop\System\Files;use Pam\Desktop\Internal\Wire;use Pam\Desktop\ModuleResultStatus;use Pam\Desktop\Modules\{NativeModules,NativeModuleTransport};
$done=null;$calls=[];NativeModules::useTransport(new class($done,$calls) implements NativeModuleTransport{
 public function __construct(private ?Closure &$done,private array &$calls){}
 public function invoke(int $requestId,string $module,string $method,string $payload,Closure $complete):void{$this->done=$complete;$this->calls[]=[$module,$method,Wire::decodeMap($payload)];}
});
function saveCheck(bool $value):void{if(!$value)throw new RuntimeException('Save assertion failed');}
$result=null;$error=null;$complete=static function(bool $v)use(&$result):void{$result=$v;};$failed=static function(string $v)use(&$error):void{$error=$v;};
Files::save('José; café','extrato.csv',$complete,$failed);saveCheck($calls[0][0]==='files'&&$calls[0][1]==='save'&&base64_decode($calls[0][2]['data'])==='José; café');$done(ModuleResultStatus::Success,Wire::map(['saved'=>false]));saveCheck($result===false);
Files::save('a','a.csv',$complete,$failed);$done(ModuleResultStatus::Success,Wire::map(['saved'=>true]));saveCheck($result===true);
Files::save('a','a.csv',$complete,$failed);$done(ModuleResultStatus::Failure,'Permission denied');saveCheck($error==='Permission denied');
foreach(['../a','a/b','a\\b',''] as $name){try{Files::save('a',$name,$complete);throw new RuntimeException('Invalid name accepted');}catch(InvalidArgumentException){}}
try{Files::save(str_repeat('a',1048577),'a.csv',$complete);throw new RuntimeException('Limit exceeded');}catch(InvalidArgumentException){}
NativeModules::useTransport(null);echo "PASS files.save native success/cancel/error, UTF8, filename and byte bounds\n";
