// Serves /obs-rest.ndjson = Observation.ndjson from line 648601 to the end (bytes 641011353..7523591228), HTTP/1.1, exact Content-Length, no copy on disk
const http=require('http'),fs=require('fs');
const SRC='/home/angela/data/Observation.ndjson', START=641011353, LEN=6882579876, MAN='/home/angela/pass-1182/run/manifest-rest.json';
http.createServer((req,res)=>{
  const p=new URL(req.url,'http://x').pathname;
  if(p==='/manifest-rest.json'){const b=fs.readFileSync(MAN);res.writeHead(200,{'Content-Type':'application/json','Content-Length':b.length});return res.end(req.method==='HEAD'?undefined:b);}
  if(p!=='/obs-rest.ndjson'){res.writeHead(404);return res.end('not found');}
  res.writeHead(200,{'Content-Type':'application/fhir+ndjson','Content-Length':LEN});
  if(req.method==='HEAD')return res.end();
  fs.createReadStream(SRC,{start:START,end:START+LEN-1}).pipe(res);
}).listen(8001,'127.0.0.1',()=>console.log('suffix server on 8001'));
