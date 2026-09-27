// 解压 DSH session jsonl.zstd（多帧拼接）并检索报错
const fs = require('fs');
const zlib = require('zlib');

const file = process.argv[2];
const pattern = process.argv[3] ? new RegExp(process.argv[3], 'i') : null;

if (!zlib.createZstdDecompress) {
  console.log('NO_ZSTD_STREAM_SUPPORT (node ' + process.version + ')');
  process.exit(2);
}

const chunks = [];
const rs = fs.createReadStream(file);
const zs = zlib.createZstdDecompress();

rs.on('error', (e) => { console.log('READ_FAILED: ' + e.message); process.exit(3); });
zs.on('error', (e) => { console.log('DECOMPRESS_FAILED: ' + e.message); });

zs.on('data', (c) => chunks.push(c));
zs.on('end', () => {
  const text = Buffer.concat(chunks).toString('utf8');
  const lines = text.split('\n').filter((l) => l.trim().length > 0);
  console.log('total lines: ' + lines.length);

  if (!pattern) {
    for (const l of lines.slice(-20)) console.log(l.slice(0, 700));
    return;
  }

  const hits = lines.filter((l) => pattern.test(l));
  console.log('matches: ' + hits.length);
  for (const h of hits.slice(-20)) console.log('---\n' + h.slice(0, 1500));
});

rs.pipe(zs);
