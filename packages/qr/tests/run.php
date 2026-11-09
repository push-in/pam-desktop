<?php

declare(strict_types=1);
require getenv('PAM_QR_TEST_AUTOLOAD') ?: dirname(__DIR__).'/vendor/autoload.php';
use BaconQrCode\Encoder\Encoder;
use Pam\Desktop\Canvas\Internal\DesktopCommands;
use Pam\Desktop\Qr\{ErrorCorrection,QrCode};
function qrCheck(bool $value,string $message):void {if(!$value)throw new RuntimeException($message);}
$payload='PAM Desktop • PIX local';
$code=new QrCode($payload,256);
$matrix=Encoder::encode($payload,ErrorCorrection::Medium->encoder(),'UTF-8')->getMatrix();
$commands=json_decode(DesktopCommands::encode($code->scene),true,flags:JSON_THROW_ON_ERROR);
$rectangles=[];
foreach($commands as $command) {
    if(($command['kind']??0)!==5)continue;
    preg_match_all('/M([0-9]+) ([0-9]+) L([0-9]+) ([0-9]+) L([0-9]+) ([0-9]+) L([0-9]+) ([0-9]+) Z/',$command['path'],$matches,PREG_SET_ORDER);
    foreach($matches as $match)$rectangles[]=[(int)$match[1],(int)$match[2],(int)$match[3],(int)$match[6]];
}
$origin=intdiv(256-$code->moduleCount*$code->moduleSize,2);
qrCheck($origin>=4*$code->moduleSize,'Mandatory quiet zone preserved.');
for($y=0;$y<$code->moduleCount;++$y)for($x=0;$x<$code->moduleCount;++$x){
    $px=$origin+$x*$code->moduleSize+.5;$py=$origin+$y*$code->moduleSize+.5;$painted=false;
    foreach($rectangles as[$left,$top,$right,$bottom])if($px>=$left&&$px<$right&&$py>=$top&&$py<$bottom){$painted=true;break;}
    qrCheck($painted===($matrix->get($x,$y)===1),'Canvas exactly preserves encoded module matrix.');
}
$start=hrtime(true);$large=new QrCode(str_repeat('7',7000),512,ErrorCorrection::Low);
qrCheck($large->moduleCount===177,'Maximum version40 supported.');
$encoded=DesktopCommands::encode($large->scene);
qrCheck(strlen($encoded)<1_048_576&&count($large->scene->commands)<1000,'Maximum QR remains within native frame/path budgets.');
foreach([['',256],[str_repeat('x',7090),256],['example',20],[str_repeat('x',1000),29]]as[$data,$size]){
    try{new QrCode($data,$size);throw new RuntimeException('Invalid QR accepted.');}catch(InvalidArgumentException){}
}
qrCheck(array_column(ErrorCorrection::cases(),'value')===[1,2,3,4],'Public error correction enum is sequential.');
$stretched=QrCode::make('PIX',240,ErrorCorrection::Low,0,true);$plain=QrCode::make('PIX',240,ErrorCorrection::Low);
qrCheck($stretched->margin===0&&$stretched->stretch&&$plain->margin===QrCode::QUIET_MODULES&&!$plain->stretch,'Margin and stretch are kept.');
$bitmap=QrCode::make('PIX',240,ErrorCorrection::Low,0,false,224);$info=getimagesizefromstring((string)$bitmap->png);
qrCheck(is_array($info)&&$info[0]===224&&$info[1]===224&&$plain->png===null,'Image mode draws a PNG of the asked pixels.');
try{QrCode::make('PIX',240,ErrorCorrection::Low,-1);qrCheck(false,'A negative margin is rejected.');}catch(InvalidArgumentException){}
echo 'QR matrix, quiet zone, Unicode, version40 and native budgets: passed; max-frame='.strlen($encoded).' bytes, encode-ms='.round((hrtime(true)-$start)/1e6,1).PHP_EOL;
