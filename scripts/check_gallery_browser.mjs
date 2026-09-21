// Optional Chromium integration check; votes only against synthetic check fixtures.
import {spawn} from 'node:child_process';
import {mkdtemp, writeFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import assert from 'node:assert/strict';
const [browser,url,screenshot,mode]=process.argv.slice(2);
if(!browser||!url)throw Error('Usage: node scripts/check_gallery_browser.mjs chromium url screenshot [inspect]');
const profile=await mkdtemp(join(tmpdir(),'topography-browser-'));
const child=spawn(browser,['--headless','--no-first-run','--no-default-browser-check','--disable-background-networking','--remote-debugging-pipe',`--user-data-dir=${profile}`],{stdio:['ignore','ignore','pipe','pipe','pipe']});
let next=0,buffer='',stderr='';const pending=new Map(),errors=[],contexts=new Map();
child.stderr.on('data',data=>{stderr=(stderr+data).slice(-10000);});
child.stdio[4].on('data',data=>{buffer+=data.toString();let end;while((end=buffer.indexOf('\0'))>=0){const message=JSON.parse(buffer.slice(0,end));buffer=buffer.slice(end+1);if(message.id){const waiter=pending.get(message.id);if(waiter){pending.delete(message.id);clearTimeout(waiter.timer);message.error?waiter.reject(Error(JSON.stringify(message.error))):waiter.resolve(message.result);}}else if(message.method==='Runtime.exceptionThrown'){errors.push(message.params.exceptionDetails);}else if(message.method==='Runtime.executionContextCreated'){const ctx=message.params.context;contexts.set(ctx.id,ctx);}else if(message.method==='Runtime.executionContextDestroyed'){contexts.delete(message.params.executionContextId);}else if(message.method==='Runtime.executionContextsCleared'){contexts.clear();}}});
function command(method,params={},sessionId){return new Promise((resolve,reject)=>{const id=++next;const timer=setTimeout(()=>{pending.delete(id);reject(Error(`CDP timeout ${method}: ${stderr}`));},20000);pending.set(id,{resolve,reject,timer});child.stdio[3].write(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})})+'\0');});}
try{
 const {targetId}=await command('Target.createTarget',{url:'about:blank'});const {sessionId}=await command('Target.attachToTarget',{targetId,flatten:true});
 const c=(method,params)=>command(method,params,sessionId);
 await c('Runtime.enable');await c('Page.enable');await c('Emulation.setDeviceMetricsOverride',{width:1440,height:980,deviceScaleFactor:1,mobile:false});
 const evaluate=async(expression,contextId)=>{const value=await c('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,...(contextId?{contextId}:{})});if(value.exceptionDetails)throw Error(JSON.stringify(value.exceptionDetails));return value.result.value;};
 const wait=async(expression)=>evaluate(`new Promise((resolve,reject)=>{const end=Date.now()+15000;function poll(){if(${expression})resolve(true);else if(Date.now()>end)reject(Error('UI condition timed out: '+${JSON.stringify(expression)}));else setTimeout(poll,50);}poll();})`);
 await c('Page.navigate',{url});await wait("document.querySelectorAll('.tile').length>0 && !document.querySelector('#yes').disabled");
 assert.equal(await evaluate("document.querySelectorAll('iframe').length"),1);
 assert.ok(await evaluate("[...document.querySelectorAll('.tile')].every(b=>b.querySelector('.state').getBoundingClientRect().bottom<=b.getBoundingClientRect().bottom)"),'thumbnail names and decisions must remain visible');
 if(mode!=='inspect'){
  assert.equal(await evaluate("document.querySelector('#filter').value"),'pending');
  await evaluate("document.querySelector('#yes').click()");await wait("document.querySelector('#message').textContent.startsWith('Saved Yes')");
  await c('Page.reload');await wait("document.querySelectorAll('.tile').length===4");
  assert.equal(await evaluate("document.querySelector('#filter').value"),'all');
  assert.equal(await evaluate("document.querySelectorAll('.tile[data-decision=accept]').length"),2);
  await evaluate("document.querySelectorAll('.tile')[1].click()");
  assert.equal(await evaluate("document.querySelector('#yes').disabled"),true,'votes disabled until the selected preview is ready');
  // Exercise the actual keyboard bridge from the sandboxed terrain inspector.
  let context;
  for(let tries=0;tries<100;tries++){
   for(const ctx of contexts.values()){
    if(ctx.auxData?.isDefault){try{if(await evaluate("location.pathname==='/preview/1' && document.readyState==='complete'",ctx.id))context=ctx.id;}catch{}}
   }
   if(context)break;await new Promise(resolve=>setTimeout(resolve,50));
  }
  assert.ok(context,'terrain iframe context');
  await evaluate("document.body.focus();dispatchEvent(new KeyboardEvent('keydown',{key:'n',bubbles:true}));",context);
  await wait("document.querySelector('#message').textContent.startsWith('Saved No')");
  await c('Page.reload');await wait("document.querySelectorAll('.tile[data-decision=reject]').length===3");
  assert.equal(await evaluate("document.querySelector('#tile-title').textContent"),'First <terrain>');
 }

 if(screenshot){const image=await c('Page.captureScreenshot',{format:'png'});await writeFile(screenshot,Buffer.from(image.data,'base64'));}
 assert.equal(errors.length,0,JSON.stringify(errors));
 console.log(mode==='inspect'?'Gallery browser inspection passed (no votes).':'Gallery browser checks passed: buttons, filters, reload, iframe keyboard and metadata escaping.');
}finally{
 child.kill();for(const waiter of pending.values())clearTimeout(waiter.timer);
 await new Promise(resolve=>child.exitCode!==null?resolve():child.once('exit',resolve));
 await rm(profile,{recursive:true,force:true});
}
