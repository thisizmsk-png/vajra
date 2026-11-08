const $=id=>document.getElementById(id);let blob,timer;
async function post(path){const r=await fetch(path,{method:'POST'});if(!r.ok)throw Error('Request refused');return r.json();}
function clear(){clearInterval(timer);timer=null;if(blob)URL.revokeObjectURL(blob);$('qr').hidden=true;$('pairLink').hidden=true;$('confirm').hidden=true;$('code').textContent='';}
async function state(){const r=await fetch('/state');const s=await r.json();if(!s.active&&timer)clear();$('status').textContent=s.paired?'Paired · read-only status':s.pending?'Waiting for desktop confirmation':s.active?'Waiting for scan':'Not paired. No relay polling.';$('confirm').hidden=!s.pending||s.paired;$('code').textContent=s.pending?'Compare on phone: '+s.pending:'';if(s.paired){$('qr').hidden=true;$('pairLink').hidden=true;}}
$('pair').onclick=async()=>{clear();try{const p=await post('/pair');blob=URL.createObjectURL(new Blob([p.svg],{type:'image/svg+xml'}));$('qr').src=blob;$('qr').hidden=false;$('pairLink').href=p.url;$('pairLink').hidden=false;timer=setInterval(()=>state().catch(()=>{}),1000);await state();}catch{$('status').textContent='Pairing unavailable';}};
$('confirm').onclick=async()=>{await post('/confirm');await state();};
$('revoke').onclick=async()=>{await post('/revoke');clear();$('status').textContent='Revoked. No relay polling.';};

state().catch(()=>{$('status').textContent='Desktop unavailable';});
