import {unb64,keypair,publicKey,derive,shared,seal,open,code,fresh,relayURL} from './crypto.mjs';
const $=id=>document.getElementById(id);let last=0,lastAt=0,key,invite,timer,busy=false;
function clearStatus(message){$('projects').replaceChildren();$('status').textContent=message;}
function render(v){
 if(v?.schema!=='vajra.mobile_status/1'||!Array.isArray(v.projects)||v.projects.length>64)throw Error('schema');
 const frag=document.createDocumentFragment();let total=0;
 for(const p of v.projects){if(typeof p.name!=='string'||p.name.length>160||!Array.isArray(p.runs)||p.runs.length>512)throw Error('schema');
  const a=document.createElement('article'),h=document.createElement('h2');h.textContent=p.name;a.append(h);
  for(const r of p.runs){if(++total>512||typeof r.id!=='string'||r.id.length>128||!['running','stopped','done','failed','unknown'].includes(r.status)||!Number.isSafeInteger(r.needs_you)||r.needs_you<0)throw Error('schema');const row=document.createElement('p');row.textContent=r.id+' · '+r.status+' · needs you: '+r.needs_you;a.append(row);}frag.append(a);
 }$('projects').replaceChildren(frag);
}
async function request(box,method='GET',value){const r=await fetch(invite.relay+'/rooms/'+invite.room+'/'+box,{method,headers:{authorization:'Bearer '+invite.cap,...(value?{'content-type':'application/json'}:{})},body:value?JSON.stringify(value):undefined,signal:AbortSignal.timeout(2000),redirect:'error',cache:'no-store'});if(!r.ok)throw Error('unavailable');const text=await r.text();if(text.length>96000)throw Error('limit');return JSON.parse(text);}
async function poll(){if(busy)return;busy=true;try{const packet=await request('snapshot');if(packet){const v=await open(key,invite.room,'snapshot',packet);const seq=fresh(v,last);render(v.snapshot);last=seq;lastAt=v.at;$('status').textContent='Connected · read-only · '+new Date(v.at).toLocaleTimeString();$('code').textContent='';}}catch{/* Repeated retained packets are rejected; never reset sequence after network failure. */}finally{if(!lastAt&&invite&&Date.now()>invite.expires){clearInterval(timer);$('code').textContent='';clearStatus('Pairing expired. Scan a new QR.');}if(lastAt&&Date.now()-lastAt>15000)clearStatus('Offline / expired / revoked, cached content cleared');busy=false;}}
try{
 const fragment=location.hash.slice(1);history.replaceState(null,'',location.pathname);if(fragment.length>3000)throw Error('invite');invite=JSON.parse(new TextDecoder().decode(unb64(fragment)));invite.relay=relayURL(invite.relay);
 if(invite.v!==1||!Number.isSafeInteger(invite.expires)||Date.now()>invite.expires||invite.expires>Date.now()+120000||!/^[A-Za-z0-9_-]{32}$/.test(invite.room)||unb64(invite.secret).length!==32||unb64(invite.cap).length!==32)throw Error('invite');
 const keys=await keypair(),pub=await publicKey(keys);key=await shared(keys,invite.host,invite.secret,invite.room);const inviteKey=await derive(invite.secret,invite.room);
 $('code').textContent='Compare on desktop: '+await code(invite.room,invite.host,pub);$('status').textContent='Waiting for desktop to confirm matching code';
 await request('hello','POST',await seal(inviteKey,invite.room,'hello',{kind:'hello',phone:pub,expires:invite.expires}));
 timer=setInterval(poll,1000);await poll();
 window.addEventListener('pagehide',()=>{clearInterval(timer);key=null;invite=null;clearStatus('Disconnected');},{once:true});
}catch{clearStatus('Pairing unavailable or expired. Scan a new desktop QR.');}
