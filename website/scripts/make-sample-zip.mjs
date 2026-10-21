// Builds public/sample.zip from the repo's sample/ corpus so the download can
// never drift from the source. Dependency-free (node:zlib); deterministic.
import { readdirSync, readFileSync, writeFileSync, statSync } from "node:fs";
import { deflateRawSync } from "node:zlib";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const src = join(here, "..", "..", "sample");
const out = join(here, "..", "public", "sample.zip");

const table = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf) => {
  let c = 0xffffffff;
  for (const b of buf) c = table[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};

const walk = (dir, rel = "") =>
  readdirSync(dir).sort().flatMap((name) => {
    const full = join(dir, name);
    const r = rel ? `${rel}/${name}` : name;
    return statSync(full).isDirectory() ? walk(full, r) : [r];
  });

const files = walk(src);
if (!files.length) throw new Error(`no files under ${src}`);
const DOS_TIME = 0, DOS_DATE = (1 << 5) | 1; // 1980-01-01
const locals = [], centrals = [];
let offset = 0;
for (const rel of files) {
  const data = readFileSync(join(src, rel));
  const comp = deflateRawSync(data);
  const name = Buffer.from(`sample/${rel}`);
  const crc = crc32(data);
  const lh = Buffer.alloc(30);
  lh.writeUInt32LE(0x04034b50, 0); lh.writeUInt16LE(20, 4); lh.writeUInt16LE(0x0800, 6);
  lh.writeUInt16LE(8, 8); lh.writeUInt16LE(DOS_TIME, 10); lh.writeUInt16LE(DOS_DATE, 12);
  lh.writeUInt32LE(crc, 14); lh.writeUInt32LE(comp.length, 18); lh.writeUInt32LE(data.length, 22);
  lh.writeUInt16LE(name.length, 26);
  const ch = Buffer.alloc(46);
  ch.writeUInt32LE(0x02014b50, 0); ch.writeUInt16LE(20, 4); ch.writeUInt16LE(20, 6);
  ch.writeUInt16LE(0x0800, 8); ch.writeUInt16LE(8, 10); ch.writeUInt16LE(DOS_TIME, 12);
  ch.writeUInt16LE(DOS_DATE, 14); ch.writeUInt32LE(crc, 16); ch.writeUInt32LE(comp.length, 20);
  ch.writeUInt32LE(data.length, 24); ch.writeUInt16LE(name.length, 28); ch.writeUInt32LE(offset, 42);
  locals.push(lh, name, comp);
  centrals.push(ch, name);
  offset += 30 + name.length + comp.length;
}
const cd = Buffer.concat(centrals);
const end = Buffer.alloc(22);
end.writeUInt32LE(0x06054b50, 0); end.writeUInt16LE(files.length, 8);
end.writeUInt16LE(files.length, 10); end.writeUInt32LE(cd.length, 12); end.writeUInt32LE(offset, 16);
writeFileSync(out, Buffer.concat([...locals, cd, end]));
console.log(`sample.zip: ${files.length} files`);
