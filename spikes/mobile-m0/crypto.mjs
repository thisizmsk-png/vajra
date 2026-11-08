// Spike protocol only, not a reviewed production handshake. Uses platform crypto, no vendored primitives.
const subtle = globalThis.crypto.subtle;
const enc = new TextEncoder(), dec = new TextDecoder();
export const b64 = b => { let s=''; for(const x of new Uint8Array(b)) s+=String.fromCharCode(x); return btoa(s).replaceAll('+','-').replaceAll('/','_').replace(/=+$/,''); };
export const unb64 = s => { if(typeof s!=='string'||s.length>100000||!/^[A-Za-z0-9_-]*$/.test(s)) throw Error('encoding'); return Uint8Array.from(atob(s.replaceAll('-','+').replaceAll('_','/')), c=>c.charCodeAt(0)); };
export const random = n => b64(crypto.getRandomValues(new Uint8Array(n)));
export const keypair = () => subtle.generateKey({name:'ECDH',namedCurve:'P-256'},false,['deriveBits']);
export const publicKey = k => subtle.exportKey('raw',k.publicKey).then(b64);
export async function derive(secret, room, material) {
 const ikm = await subtle.importKey('raw',material ? material : unb64(secret),'HKDF',false,['deriveKey']);
 return subtle.deriveKey({name:'HKDF',hash:'SHA-256',salt:unb64(secret),info:enc.encode('vajra-mobile-m0/1|'+room)},ikm,{name:'AES-GCM',length:256},false,['encrypt','decrypt']);
}
export async function shared(keys, peer, secret, room) {
 const p=await subtle.importKey('raw',unb64(peer),{name:'ECDH',namedCurve:'P-256'},false,[]);
 const dh=await subtle.deriveBits({name:'ECDH',public:p},keys.privateKey,256);
 return derive(secret,room,dh);
}
export async function seal(key,room,direction,value) {
 const iv=crypto.getRandomValues(new Uint8Array(12));
 const ciphertext=await subtle.encrypt({name:'AES-GCM',iv,additionalData:enc.encode(room+'|'+direction),tagLength:128},key,enc.encode(JSON.stringify(value)));
 return {v:1,iv:b64(iv),ciphertext:b64(ciphertext)};
}
export async function open(key,room,direction,packet) {
 if(packet?.v!==1||unb64(packet.iv).length!==12) throw Error('envelope');
 return JSON.parse(dec.decode(await subtle.decrypt({name:'AES-GCM',iv:unb64(packet.iv),additionalData:enc.encode(room+'|'+direction),tagLength:128},key,unb64(packet.ciphertext))));
}
export async function code(room,host,phone) {return b64(await subtle.digest('SHA-256',enc.encode('vajra-mobile-m0/1|'+room+'|'+host+'|'+phone))).slice(0,12);}
export function fresh(value,last,now=Date.now()) {
 if(!Number.isSafeInteger(value.seq)||value.seq<=last||!Number.isSafeInteger(value.at)||Math.abs(now-value.at)>15000||value.kind!=='snapshot') throw Error('stale_or_replay');
 return value.seq;
}
export function relayURL(url) {
 const u=new URL(url); if(u.username||u.password||u.search||u.hash||u.pathname!=='/'||!(u.protocol==='https:'||(u.protocol==='http:'&&['127.0.0.1','localhost','[::1]'].includes(u.hostname))))throw Error('relay_requires_https'); return u.origin;
}
