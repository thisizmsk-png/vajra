import http from 'node:http';
import { readFile, lstat } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { random,keypair,publicKey,derive,shared,seal,open,code,relayURL } from './crypto.mjs';
import { headers } from './relay.mjs';
const root=new URL('./',import.meta.url);
const CSP="default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self' http://127.0.0.1:*; base-uri 'none'; frame-ancestors 'none'; form-action 'none'";
export function validateSnapshot(v){
 if(v?.schema!=='vajra.mobile_status/1'||!Array.isArray(v.projects)||v.projects.length>64)throw Error('schema');
 return {schema:v.schema,projects:v.projects.map(p=>{
  if(typeof p.id!=='string'||p.id.length>128||typeof p.name!=='string'||p.name.length>160||!Array.isArray(p.runs)||p.runs.length>512)throw Error('project');
  return {id:p.id,name:p.name,runs:p.runs.map(r=>{if(typeof r.id!=='string'||r.id.length>128||!['running','stopped','done','failed','unknown'].includes(r.status)||!Number.isSafeInteger(r.needs_you)||r.needs_you<0||r.needs_you>100000)throw Error('run');return {id:r.id,status:r.status,needs_you:r.needs_you};})};})};
}
export async function createHost({relay,snapshotFile,port=8787}){
 relay=relayURL(relay);let pair=null,timer=null,busy=false;const csrf=random(32);let ownerOrigin;
 async function relayCall(path,method='GET',value,cap){const r=await fetch(relay+path,{method,headers:{'content-type':'application/json',...(cap?{authorization:'Bearer '+cap}:{})},body:value===undefined?undefined:JSON.stringify(value),signal:AbortSignal.timeout(2000),redirect:'error'});if(!r.ok)throw Error('relay');return r.status===204?null:r.json();}
 async function revoke(){const p=pair;pair=null;clearInterval(timer);timer=null;if(p)try{await relayCall('/rooms/'+p.room.id,'DELETE',undefined,p.room.host);}catch{/* Local key destruction must not depend on relay. */}}
 async function tick(){if(busy||!pair)return;busy=true;const p=pair;try{
  if(p.approved&&Date.now()>p.sessionUntil){await revoke();return;}
  if(!p.approved){if(Date.now()>p.expires){await revoke();return;}if(p.pending)return;
   const packet=await relayCall('/rooms/'+p.room.id+'/hello','GET',undefined,p.room.host);if(!packet)return;
   const hello=await open(p.inviteKey,p.room.id,'hello',packet);
   if(hello.kind!=='hello'||hello.expires!==p.expires||typeof hello.phone!=='string')throw Error('hello');
   const key=await shared(p.keys,hello.phone,p.secret,p.room.id);if(pair!==p)return;
   p.pending={key,code:await code(p.room.id,p.pub,hello.phone)};
  }else{
   const st=await lstat(snapshotFile);if(!st.isFile()||st.isSymbolicLink()||st.size>65536)throw Error('snapshot');
   const snapshot=validateSnapshot(JSON.parse(await readFile(snapshotFile,'utf8')));
   const packet=await seal(p.pending.key,p.room.id,'snapshot',{kind:'snapshot',seq:++p.seq,at:Date.now(),snapshot});
   if(pair===p)await relayCall('/rooms/'+p.room.id+'/snapshot','POST',packet,p.room.host);
  }
 }catch{if(pair===p)p.error='Status unavailable';}finally{busy=false;}}
 const server=http.createServer(async(req,res)=>{const send=(status,value,type='application/json')=>{res.writeHead(status,{...headers,'content-type':type,'content-security-policy':CSP,'referrer-policy':'no-referrer','cross-origin-resource-policy':'same-origin'});res.end(typeof value==='string'?value:JSON.stringify(value));};try{
  if(req.headers.host!==new URL(ownerOrigin).host)return send(403,{});
  const u=new URL(req.url,ownerOrigin);
  if(req.method==='GET'&&u.pathname==='/'){res.setHeader('set-cookie',`m0_owner=${csrf}; HttpOnly; SameSite=Strict; Path=/`);return send(200,await readFile(new URL('desktop.html',root),'utf8'),'text/html');}
  const assets={'/desktop.mjs':['desktop.mjs','text/javascript'],'/client.mjs':['client.mjs','text/javascript'],'/crypto.mjs':['crypto.mjs','text/javascript'],'/style.css':['style.css','text/css'],'/phone':['phone.html','text/html']};
  if(req.method==='GET'&&assets[u.pathname]){const [f,t]=assets[u.pathname];return send(200,await readFile(new URL(f,root),'utf8'),t);}
  const owner=req.headers.cookie?.split('; ').includes('m0_owner='+csrf);
  if(!owner)return send(403,{});
  if(req.method==='GET'&&u.pathname==='/state')return send(200,{active:!!pair,paired:!!pair?.approved,pending:pair?.pending?.code||null,error:pair?.error||null});
  if(req.method!=='POST'||req.headers.origin!==ownerOrigin)return send(403,{});
  if(u.pathname==='/pair'){
   await revoke();const room=await relayCall('/rooms','POST',{});const keys=await keypair(),pub=await publicKey(keys),secret=random(32);const expires=Date.now()+120000;
   const p={room,keys,pub,secret,expires,inviteKey:await derive(secret,room.id),seq:0,approved:false};pair=p;
   const invite={v:1,relay,room:room.id,cap:room.phone,host:pub,secret,expires};const fragment=Buffer.from(JSON.stringify(invite)).toString('base64url');
   const url=ownerOrigin+'/phone#'+fragment;
   // qrencode is an installed local utility; input is stdin, never a shell command or remote QR service.
   const svg=await qr(url);timer=setInterval(tick,1000);timer.unref();return send(200,{url,svg,expires});
  }
  if(u.pathname==='/confirm'){const p=pair;if(!p?.pending||Date.now()>p.expires)return send(409,{});p.approved=true;p.sessionUntil=Date.now()+3600000;await tick();return send(200,{});}
  if(u.pathname==='/revoke'){await revoke();return send(200,{});}
  return send(404,{});
 }catch{return send(400,{error:'request_refused'});}});
 server.requestTimeout=5000;server.headersTimeout=5000;server.maxConnections=8;
 await new Promise(resolve=>server.listen(port,'127.0.0.1',resolve));ownerOrigin='http://127.0.0.1:'+server.address().port;
 return {server,origin:ownerOrigin,revoke,tick,state:()=>({paired:!!pair?.approved,timer:!!timer})};
}
async function qr(value){const {spawn}=await import('node:child_process');return new Promise((resolve,reject)=>{const p=spawn('qrencode',['-t','SVG','-o','-'],{stdio:['pipe','pipe','ignore']});let out='';p.stdout.on('data',b=>{out+=b;if(out.length>1000000)p.kill();});p.on('error',reject);p.on('exit',n=>n===0?resolve(out):reject(Error('qr')));p.stdin.end(value);});}
if(process.argv[1]===fileURLToPath(import.meta.url)){const h=await createHost({relay:process.env.M0_RELAY||'http://127.0.0.1:8788',snapshotFile:process.env.M0_SNAPSHOT,port:Number(process.env.M0_HOST_PORT||8787)});console.log('Desktop demo: '+h.origin);if(process.send)process.send({origin:h.origin});process.on('message',m=>{if(m==='usage'&&process.send)process.send({usage:{rss:process.memoryUsage().rss,heap:process.memoryUsage().heapUsed,cpu:process.cpuUsage()}});});process.on('SIGTERM',async()=>{await h.revoke();h.server.close(()=>process.exit());});}
