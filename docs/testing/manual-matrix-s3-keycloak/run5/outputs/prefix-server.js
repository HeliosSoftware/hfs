// Serves /obs-head.ndjson = the first 6500000 lines (6354508616 bytes) of Observation.ndjson, HTTP/1.1 with exact Content-Length, no copy on disk.
const http=require('http'),fs=require('fs');
const SRC='/home/angela/data/Observation.ndjson', LEN=6354508616, MAN='/home/angela/pass-1178/run5/manifest-obs-head.json';
http.createServer((req,res)=>{
  const p=new URL(req.url,'http://x').pathname;
  if(p==='/manifest-obs-head.json'){const b=fs.readFileSync(MAN);res.writeHead(200,{'Content-Type':'application/json','Content-Length':b.length});return res.end(req.method==='HEAD'?undefined:b);}
  if(p!=='/obs-head.ndjson'){res.writeHead(404);return res.end('not found');}
  res.writeHead(200,{'Content-Type':'application/fhir+ndjson','Content-Length':LEN});
  if(req.method==='HEAD')return res.end();
  fs.createReadStream(SRC,{start:0,end:LEN-1}).pipe(res);
}).listen(8001,'127.0.0.1',()=>console.log('prefix server on 8001, '+LEN+' bytes'));
