<?php

declare(strict_types=1);
$loader=require getenv('PAM_REALTIME_TEST_AUTOLOAD')?:dirname(__DIR__).'/vendor/autoload.php';
$loader->addPsr4('Pam\\Desktop\\Realtime\\',dirname(__DIR__).'/src');
use Pam\Desktop\Realtime\{ReverbConfig,ReverbSubscription,ConnectionState};
use Pam\Desktop\Modules\{NativeModules,NativeModuleTransport};
use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\Http\{Http,HttpResponse,Request};
function realtimeCheck(bool $ok,string $message):void{if(!$ok)throw new RuntimeException($message);}
$transport=new class implements NativeModuleTransport{
 public array $pending=[];public array $sent=[];public array $closed=[];public array $timers=[];private int $next=1;
 public function invoke(int $requestId,string $module,string $method,string $payload,Closure $complete):void{
  $data=Wire::decodeMap($payload);
  if($module==='timers'){if($method==='after')$this->timers[$data['timer']]=$complete;else unset($this->timers[$data['timer']]);return;}
  if($module!=='websocket')throw new RuntimeException('Unexpected module '.$module);
  if($method==='open'){$complete(ModuleResultStatus::Success,Wire::map(['id'=>'socket-'.$this->next++]));return;}
  if($method==='next'){$this->pending[$data['id']]=$complete;return;}
  if($method==='send'){$this->sent[]=json_decode($data['text'],true);$complete(ModuleResultStatus::Success,'');return;}
  if($method==='close'){$this->closed[]=$data['id'];unset($this->pending[$data['id']]);$complete(ModuleResultStatus::Success,'');}
 }
 public function emit(string $id,int $kind,array $frame=[]):void{$callback=$this->pending[$id];unset($this->pending[$id]);$callback(ModuleResultStatus::Success,Wire::map(['kind'=>$kind,'text'=>$frame===[]?'':json_encode($frame)]));}
};
NativeModules::useTransport($transport);$requests=[];
$http=Http::create('https://api.test/api')->withHeaders(['Authorization'=>'Bearer fixture-only'])->withTransport(static function(Request $request,Closure $complete)use(&$requests):Closure{$requests[]=['request'=>$request,'complete'=>$complete];return static function():void{};});
$config=new ReverbConfig('https://api.test/api','wss://realtime.test','public_key');
realtimeCheck($config->authorizationUrl==='https://api.test/broadcasting/auth','Auth only targets trusted API origin, not socket host.');
foreach([['https://api.test','ws://external.test','key'],['https://user:secret@api.test','wss://rt.test','key']]as$args){try{new ReverbConfig(...$args);throw new RuntimeException('Unsafe config accepted');}catch(InvalidArgumentException){}}
$states=[];$messages=[];
$subscribe=static function()use($config,$http,&$messages,&$states){return ReverbSubscription::privateChannel($config,$http,'store.8',['order.status.updated'],static function(string $name,array $data)use(&$messages):void{$messages[]=$data;},static function(ConnectionState $state)use(&$states):void{$states[]=$state;});};
$sub=$subscribe();$transport->emit('socket-1',1);$transport->emit('socket-1',2,['event'=>'pusher:connection_established','data'=>json_encode(['socket_id'=>'123.456','activity_timeout'=>120])]);
realtimeCheck(count($requests)===1&&$requests[0]['request']->url==='https://api.test/broadcasting/auth','Private authorization uses trusted origin.');
realtimeCheck(json_decode($requests[0]['request']->body,true)===['socket_id'=>'123.456','channel_name'=>'private-store.8'],'Reverb private protocol body.');
realtimeCheck($transport->sent===[],'No subscription before HTTP authorization success.');
($requests[0]['complete'])(new HttpResponse(200,json_encode(['auth'=>'public_key:'.str_repeat('a',64)])));
realtimeCheck($transport->sent[0]['event']==='pusher:subscribe','Authorized private subscribe.');
$transport->emit('socket-1',2,['event'=>'pusher_internal:subscription_succeeded','channel'=>'private-store.8','data'=>'{}']);realtimeCheck($sub->state()===ConnectionState::Subscribed,'Only acknowledged subscription is live.');
$transport->emit('socket-1',2,['event'=>'order.status.updated','channel'=>'private-store.9','data'=>['id'=>1]]);realtimeCheck($messages===[],'Other-store events ignored.');
$transport->emit('socket-1',2,['event'=>'order.status.updated','channel'=>'private-store.8','data'=>['id'=>1]]);realtimeCheck(count($messages)===1,'Allowlisted selected-store event delivered.');
$sub->suspend();realtimeCheck($transport->pending===[]&&$transport->timers===[],'Suspension releases socket, reader and timers.');$sub->resume();realtimeCheck(isset($transport->pending['socket-2']),'Resume opens a new socket.');$sub->cancel();realtimeCheck($sub->disposed()&&$transport->timers===[],'Dispose owns all resources.');
$sub=$subscribe();$transport->emit('socket-3',1);$transport->emit('socket-3',2,['event'=>'pusher:connection_established','data'=>['socket_id'=>'1.2','activity_timeout'=>120]]);($requests[1]['complete'])(new HttpResponse(403,'{}'));realtimeCheck($sub->state()===ConnectionState::Rejected&&$transport->timers===[],'403 disables channel without automatic credential retries.');$sub->cancel();
NativeModules::useTransport(null);echo "Reverb trusted-origin auth, private protocol, scope, lifecycle and denied-auth tests passed.\n";
