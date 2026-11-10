// Opaque in-memory mailbox. TLS termination is required for non-loopback deployment; no public bind in spike.
import http from 'node:http';
import { random } from './crypto.mjs';
export const headers={'content-type':'application/json','cache-control':'no-store','x-content-type-options':'nosniff'};
export async function body(req) {let n=0,chunks=[];for await(const c of req){n+=c.length;if(n>96000)throw Error('limit');chunks.push(c);}return JSON.parse(Buffer.concat(chunks).toString());}
export function createRelay(){
 const rooms=new Map();let window=Date.now(),hits=0;
 const server=http.createServer(async(req,res)=>{try{
  if(Date.now()-window>1000){window=Date.now();hits=0;}if(++hits>40)throw Error('rate');
  const u=new URL(req.url,'http://localhost'); const parts=u.pathname.split('/');
  const send=(status,value)=>{res.writeHead(status,headers);res.end(JSON.stringify(value));};
  // Explicit local origin: clients in spike are served by the desktop on this same machine.
  const origin=req.headers.origin;
  if(origin){if(!/^http:\/\/(127\.0\.0\.1|localhost):\d+$/.test(origin))return send(403,{});res.setHeader('access-control-allow-origin',origin);res.setHeader('vary','Origin');}
  if(req.method==='OPTIONS'){res.setHeader('access-control-allow-methods','GET, POST, DELETE');res.setHeader('access-control-allow-headers','authorization, content-type');return send(204,null);}
  for(const [id,r] of rooms)if(Date.now()>r.expires)rooms.delete(id);
  if(u.pathname==='/rooms'&&req.method==='POST'){
   if(rooms.size>=8)return send(429,{});const id=random(24),host=random(32),phone=random(32);rooms.set(id,{host,phone,expires:Date.now()+3600000,hello:null,snapshot:null});return send(201,{id,host,phone});
  }
  const r=rooms.get(parts[2]);if(!r)return send(404,{});
  const token=req.headers.authorization?.replace(/^Bearer /,'');const role=token===r.host?'host':token===r.phone?'phone':null;if(!role)return send(403,{});
  if(req.method==='DELETE'&&role==='host'){rooms.delete(parts[2]);return send(200,{});}
  if(parts.length!==4||!['hello','snapshot'].includes(parts[3]))return send(404,{});
  const box=parts[3];if(req.method==='GET'){if((box==='hello')!==(role==='host'))return send(403,{});return send(200,r[box]);}
  if(req.method==='POST'){
   if((box==='hello')!==(role==='phone'))return send(403,{});
   if(box==='hello'&&r.hello)return send(409,{});
   const v=await body(req);if(v?.v!==1||typeof v.iv!=='string'||typeof v.ciphertext!=='string'||Object.keys(v).sort().join(',')!=='ciphertext,iv,v')return send(400,{});
   r[box]=v;return send(200,{});
  }return send(405,{});
 }catch{res.writeHead(400,headers);res.end('{}');}});
 return {server,rooms};
}
if(process.argv[1]===new URL(import.meta.url).pathname){const {server}=createRelay();server.listen(Number(process.env.M0_RELAY_PORT||8788),'127.0.0.1',()=>console.log('relay listening on loopback'));}
