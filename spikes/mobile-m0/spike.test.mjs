import {test} from 'node:test';import assert from 'node:assert/strict';import {mkdtemp,writeFile,rm} from 'node:fs/promises';import {tmpdir} from 'node:os';import {join} from 'node:path';
import {createRelay} from './relay.mjs';import {createHost,validateSnapshot} from './host.mjs';import {random,keypair,publicKey,derive,shared,seal,open,code,fresh,relayURL} from './crypto.mjs';
test('AEAD binds key, room and direction; detects tamper; replay/expiry rejected',async()=>{
 const room=random(24),secret=random(32),a=await keypair(),b=await keypair();const ap=await publicKey(a),bp=await publicKey(b),ak=await shared(a,bp,secret,room),bk=await shared(b,ap,secret,room);
 const packet=await seal(ak,room,'snapshot',{kind:'snapshot',seq:1,at:Date.now()});const v=await open(bk,room,'snapshot',packet);assert.equal(fresh(v,0),1);assert.throws(()=>fresh(v,1));assert.throws(()=>fresh({...v,at:Date.now()-16000},0));
 await assert.rejects(open(bk,room,'hello',packet));await assert.rejects(open(bk,'different','snapshot',packet));await assert.rejects(open(await derive(random(32),room),room,'snapshot',packet));
 await assert.rejects(open(bk,room,'snapshot',{...packet,ciphertext:packet.ciphertext.slice(0,-4)+'AAAA'}));assert.equal(await code(room,ap,bp),await code(room,ap,bp));
 assert.throws(()=>relayURL('http://example.org'));assert.throws(()=>relayURL('https://example.org/a'));assert.throws(()=>validateSnapshot({schema:'vajra.mobile_status/1',projects:[{id:'p',name:'x',runs:[{id:'r',status:'execute',needs_you:0}]}]}));
});
test('QR invite → encrypted hello → desktop confirm → encrypted status; mutation denied; revoke tears down',async()=>{
 assert.ok(process.env.HOME.startsWith('/tmp/vajra-mobile-')||process.env.HOME.startsWith('/private/tmp/vajra-mobile-'),'test HOME must be temporary');
 const dir=await mkdtemp(join(tmpdir(),'vajra-mobile-fixture-')),file=join(dir,'status.json');
 const fixture={schema:'vajra.mobile_status/1',projects:[{id:'p',name:'Fixture Project',root:'/must-not-leave',runs:[{id:'r',status:'running',needs_you:2,command:'never-export'}]}]};await writeFile(file,JSON.stringify(fixture));
 const relay=createRelay();await new Promise(r=>relay.server.listen(0,'127.0.0.1',r));const relayOrigin='http://127.0.0.1:'+relay.server.address().port;
 const host=await createHost({relay:relayOrigin,snapshotFile:file,port:0});
 try {
  assert.equal(host.state().timer,false);const page=await fetch(host.origin);const cookie=page.headers.get('set-cookie').split(';')[0];await page.text();
  const post=path=>fetch(host.origin+path,{method:'POST',headers:{cookie,origin:host.origin}});
  assert.equal((await fetch(host.origin+'/pair',{method:'POST'})).status,403);
  const response=await post('/pair');assert.equal(response.status,200);const p=await response.json();assert.ok(p.svg.includes('<svg'));
  const invite=JSON.parse(Buffer.from(new URL(p.url).hash.slice(1),'base64url'));const keys=await keypair(),pub=await publicKey(keys),key=await shared(keys,invite.host,invite.secret,invite.room);
  const ph={authorization:'Bearer '+invite.cap,'content-type':'application/json'};const url=relayOrigin+'/rooms/'+invite.room;
  const packet=await seal(await derive(invite.secret,invite.room),invite.room,'hello',{kind:'hello',phone:pub,expires:invite.expires});
  assert.equal((await fetch(url+'/hello',{method:'POST',headers:ph,body:JSON.stringify(packet)})).status,200);
  await host.tick();let state=await (await fetch(host.origin+'/state',{headers:{cookie}})).json();assert.equal(state.pending,await code(invite.room,invite.host,pub));assert.equal(state.paired,false);
  assert.equal((await fetch(url+'/snapshot',{headers:ph})).status,200);assert.equal(await (await fetch(url+'/snapshot',{headers:ph})).json(),null);
  assert.equal((await post('/confirm')).status,200);await host.tick();
  const encrypted=await (await fetch(url+'/snapshot',{headers:ph})).json();assert.ok(!JSON.stringify(encrypted).includes('Fixture'));const value=await open(key,invite.room,'snapshot',encrypted);assert.equal(value.snapshot.projects[0].runs[0].needs_you,2);assert.ok(!JSON.stringify(value).includes('must-not-leave'));assert.ok(!JSON.stringify(value).includes('never-export'));
  assert.equal((await fetch(url+'/snapshot',{method:'POST',headers:ph,body:JSON.stringify(packet)})).status,403);
  assert.equal((await fetch(host.origin+'/approve',{method:'POST',headers:ph})).status,403);
  assert.equal((await post('/revoke')).status,200);assert.equal(host.state().timer,false);assert.equal(relay.rooms.size,0);assert.equal((await fetch(url+'/snapshot',{headers:ph})).status,404);
 }finally{await host.revoke();await new Promise(r=>host.server.close(r));await new Promise(r=>relay.server.close(r));await rm(dir,{recursive:true,force:true});}
});
