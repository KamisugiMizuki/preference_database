// Run after npm run tauri build -- --debug. Uses a private fixture workspace.
import assert from 'node:assert/strict';
import {spawn, execFileSync} from 'node:child_process';
import {mkdtempSync, mkdirSync, copyFileSync, writeFileSync, readFileSync, existsSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {resolve, join} from 'node:path';
import net from 'node:net';
import http from 'node:http';

const root = resolve(import.meta.dirname, '..');
const scratch = mkdtempSync(join(tmpdir(), 'prefdb-live-'));
mkdirSync(join(scratch, 'src-tauri'), {recursive: true});
writeFileSync(join(scratch, 'package.json'), '{}');
copyFileSync(join(root, 'src-tauri/tauri.conf.json'), join(scratch, 'src-tauri/tauri.conf.json'));
const exe = join(scratch, 'src-tauri/app.exe');
copyFileSync(join(root, 'src-tauri/target/debug/preference-database.exe'), exe);
const png = join(scratch, 'fixture.png');
writeFileSync(png, Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jG2sAAAAASUVORK5CYII=', 'base64'));
const portServer = net.createServer();
await new Promise(r => portServer.listen(0, '127.0.0.1', r));
const port = portServer.address().port;
await new Promise(r => portServer.close(r));
const child = spawn(exe, [], {cwd: scratch, windowsHide:true, env: {...process.env,
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`,
  WEBVIEW2_USER_DATA_FOLDER:join(scratch, 'webview'),
}, stdio:'ignore'});
let ws; const pending = new Map(); let id = 0;
const delay = ms => new Promise(r => setTimeout(r, ms));
async function cdp(method, params = {}) {
  const key = ++id;
  const result = new Promise((resolve, reject) => {
    const timer = setTimeout(() => {pending.delete(key);reject(new Error('CDP timeout: '+method));}, 15000);
    pending.set(key, {resolve:v=>{clearTimeout(timer);resolve(v);}, reject});
  });
  ws.send(JSON.stringify({id:key, method, params})); return result;
}
async function evaluate(expression) {
  const r = await cdp('Runtime.evaluate', {expression, awaitPromise:true, returnByValue:true});
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
  return r.result.value;
}
async function until(expression) {
  for (let i=0;i<100;i++) { if(await evaluate(expression)) return; await delay(100); }
  throw new Error('condition timed out: '+expression);
}
async function invoke(command, args={}) {
  return evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)})`);
}
const passed=[];
try {
  let target;
  for (let i=0;i<100;i++) {
    try { const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json(); target=list.find(x=>x.type==='page'&&x.webSocketDebuggerUrl); if(target)break; } catch {}
    await delay(100);
  }
  assert.ok(target,'Tauri WebView2 did not expose CDP');
  ws = new WebSocket(target.webSocketDebuggerUrl);
  ws.addEventListener('message',event=>{ const message=JSON.parse(event.data); const p=pending.get(message.id); if(p){pending.delete(message.id); message.error?p.reject(message.error):p.resolve(message.result);} });
  await new Promise((r,j)=>{ws.addEventListener('open',r,{once:true});ws.addEventListener('error',j,{once:true});});
  await until(`document.querySelector('#entry-genre')?.options.length > 0`);
  const payload = '<img data-injected src=x onerror="window.__xss=1">中文';
  const genre = await invoke('create_genre',{name:payload});
  await evaluate('location.reload(); true');
  await until(`document.querySelector('#entry-genre')?.options.length > 0`);
  assert.equal(await evaluate(`document.querySelectorAll('#genre-filter [data-injected]').length`),0);
  passed.push('custom genre HTML renders as text');
  const arrivals=[];
  const imageServer=http.createServer((req,res)=>{
    arrivals.push(performance.now());
    if(req.url==='/redirect') { res.writeHead(302,{Location:'/image'}); res.end(); }
    else {res.writeHead(200,{'Content-Type':'image/png'});res.end(readFileSync(png));}
  });
  await new Promise(r=>imageServer.listen(0,'127.0.0.1',r));
  try {
    const imageUrl=`http://127.0.0.1:${imageServer.address().port}`;
    const first=await invoke('fetch_cover_preview',{url:imageUrl+'/redirect'});
    const second=await invoke('fetch_cover_preview',{url:imageUrl+'/image'});
    assert.ok(first.startsWith('data:image/png;base64,') && second===first);
    assert.equal(arrivals.length,3);
    assert.ok(arrivals[1]-arrivals[0]>=1000 && arrivals[2]-arrivals[1]>=1000,
      'preview and redirect must observe 1 second host interval: '+JSON.stringify(arrivals.slice(1).map((time,i)=>time-arrivals[i])));
    passed.push('preview and redirect are throttled through real IPC and HTTP');
  } finally {await new Promise(r=>imageServer.close(r));}
  await evaluate(`(() => {
    document.getElementById('btn-new').click();
    document.getElementById('entry-name').value=${JSON.stringify(payload)};
    document.getElementById('entry-genre').value=${JSON.stringify(genre.id)};
    document.getElementById('entry-review').value='真实表单到数据库的行为验证文本';
    document.getElementById('entry-tags').value=${JSON.stringify(payload)};
    document.getElementById('btn-add-link').click();
    document.querySelector('.link-url').value='https://example.com/?a=1&b=2';
    document.querySelector('.link-label').value=${JSON.stringify(payload)};
    document.getElementById('entry-form').dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));
    return true;
  })()`);
  await until(`document.querySelectorAll('.entry-card').length === 1`);
  const query={keyword:null, search_field:null,genre_ids:[],ratings:[],tag_filter:[],year:null,sort_by:'updated_at',sort_order:'desc',offset:0,limit:10};
  const summaries=await invoke('get_entries',{query});
  assert.equal(summaries.length,1); const entryId=summaries[0].id;
  assert.equal((await invoke('get_entry',{id:entryId})).name,payload);
  await evaluate(`document.querySelector('.entry-card').click(); true`);
  await until(`document.querySelector('#detail-links a') !== null`);
  assert.equal(await evaluate(`document.querySelector('#detail-links a').getAttribute('href')`),'https://example.com/?a=1&b=2');
  assert.equal(await evaluate(`document.querySelector('#detail-links a').rel`),'noopener noreferrer');
  assert.equal(await evaluate(`Boolean(window.__xss) || document.querySelectorAll('[data-injected]').length > 0`),false);
  passed.push('real form -> Rust -> SQLite -> safe detail rendering');
  const stored=await invoke('import_local_image',{sourcePath:png,title:'cover',creator:null});
  await invoke('add_entry_image',{entryId,path:stored,isPrimary:true});
  assert.ok((await invoke('get_image_base64',{path:stored})).length>0);
  passed.push('local image copy, association and read through real IPC');
  // Saving invalid input reaches the real backend; retry must not reuse deleted staged files.
  const staged=await invoke('import_local_image',{sourcePath:png,title:'retry',creator:null});
  await evaluate(`(() => {
    document.getElementById('btn-new').click();
    document.getElementById('entry-name').value='retry';
    document.getElementById('entry-review').value='短';
    const item=document.createElement('div'); item.className='image-item';
    item.dataset.path=${JSON.stringify(staged)};
    document.getElementById('images-container').append(item);
    document.getElementById('entry-form').dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));
    return true;
  })()`);
  await until(`!document.getElementById('btn-save-entry').disabled`);
  assert.equal(existsSync(join(scratch,staged)),false);
  assert.equal(await evaluate(`document.querySelectorAll('#images-container .image-item:not([data-id])').length`),0,
    'failed save must remove missing staged images from retry form');
  await evaluate(`(() => {
    document.getElementById('entry-review').value='修正后的评价文本有足够字符';
    document.getElementById('entry-form').dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));
    return true;
  })()`);
  await until(`document.getElementById('modal-entry').classList.contains('hidden')`);
  const retryEntries=await invoke('get_entries',{query});
  assert.equal(retryEntries.length,2);
  await invoke('delete_entries',{ids:retryEntries.filter(e=>e.id!==entryId).map(e=>e.id)});
  passed.push('failed save cleans staged paths and corrected form can retry');
  const original=await invoke('get_entry',{id:entryId});
  const updated=await invoke('update_entry',{req:{...original,name:'edited',new_image_paths:[],removed_image_ids:[],links:original.links,tags:original.tags}});
  assert.equal(updated.name,'edited');
  const backup=await invoke('backup_database'); assert.ok(existsSync(backup));
  assert.equal(readFileSync(backup).subarray(0,16).toString(),'SQLite format 3\0');
  passed.push('update and SQLite online backup');
  const bad=await evaluate(`window.__TAURI_INTERNALS__.invoke('get_image_base64',{path:${JSON.stringify(png)}}).then(()=>false,()=>true)`);
  assert.equal(bad,true); passed.push('arbitrary file read rejected through real IPC');
  await invoke('delete_entries',{ids:[entryId]});
  assert.equal(await invoke('get_entries_count',{query:null}),0);
  assert.equal(existsSync(join(scratch,stored)),false);
  passed.push('delete cascades and removes unreferenced managed image');
  const report={passed,failed:[],scratch};
  writeFileSync(join(scratch,'result.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify(report,null,2));
} finally {
  if(ws) ws.close();
  if(child.pid) try {execFileSync('taskkill',['/PID',String(child.pid),'/T','/F'],{stdio:'ignore'});} catch {}
}
