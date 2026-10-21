#!/usr/bin/env node
// No TeX engine, shell, network, or file-loading TeX packages are exposed to math.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const {mathjax} = require('mathjax-full/js/mathjax.js');
const {TeX} = require('mathjax-full/js/input/tex.js');
const {SVG} = require('mathjax-full/js/output/svg.js');
const {liteAdaptor} = require('mathjax-full/js/adaptors/liteAdaptor.js');
const {RegisterHTMLHandler} = require('mathjax-full/js/handlers/html.js');
require('mathjax-full/js/input/tex/ams/AmsConfiguration.js');
require('mathjax-full/js/input/tex/newcommand/NewcommandConfiguration.js');
require('mathjax-full/js/input/tex/boldsymbol/BoldsymbolConfiguration.js');
const sharp = require('sharp');

async function render(input, requestedColumns) {
  const source = input.trim();
  const dir = process.env.CODEX_LATEX_CACHE;
  const columns = Math.min(240, Number(requestedColumns));
  if (!dir || !source || source.length > 8192 || columns < 22) throw Error('Unsupported size');
  const {cw, ch} = JSON.parse(fs.readFileSync(path.join(dir, 'geometry.json'), 'utf8'));
  if (!(cw >= 4 && cw <= 64 && ch >= 8 && ch <= 128)) throw Error('Invalid geometry');
  const id = crypto.createHash('sha256').update(JSON.stringify(["transparent-v2", source, columns, cw, ch])).digest('hex').slice(0, 12);
  const file = path.join(dir, id + '.png');
  const meta = path.join(dir, id + '.json');
  let rows;
  if (fs.existsSync(meta) && fs.existsSync(file)) {
    ({rows} = JSON.parse(fs.readFileSync(meta, 'utf8')));
  } else {
    const adaptor = liteAdaptor();
    RegisterHTMLHandler(adaptor);
    const tex = new TeX({packages: ['base', 'ams', 'newcommand', 'boldsymbol'], maxBuffer: 8192, maxMacros: 500});
    const document = mathjax.document('', {InputJax: tex, OutputJax: new SVG({fontCache: 'none'})});
    const result = document.convert(source, {display: true});
    const svg = adaptor.firstChild(result);
    if (adaptor.outerHTML(svg).includes('data-mjx-error')) throw Error('Invalid LaTeX');
    const ex = ch * 0.62;
    const naturalW = Math.ceil(parseFloat(adaptor.getAttribute(svg, 'width')) * ex);
    const naturalH = Math.ceil(parseFloat(adaptor.getAttribute(svg, 'height')) * ex);
    if (!(naturalW > 0 && naturalH > 0)) throw Error('Empty formula');
    const scale = Math.min(1, (columns * cw - 12) / naturalW, (18 * ch - 12) / naturalH);
    const w = Math.max(1, Math.ceil(naturalW * scale));
    const h = Math.max(1, Math.ceil(naturalH * scale));
    adaptor.setAttribute(svg, 'width', String(w));
    adaptor.setAttribute(svg, 'height', String(h));
    const bytes = Buffer.from(adaptor.outerHTML(svg).replaceAll('currentColor', '#ffffff'));
    rows = Math.ceil((h + 12) / ch);
    const width = Math.max(20 * cw, w + 12);
    const formula = await sharp(bytes, {limitInputPixels: 16000000}).png().toBuffer();
    await sharp({create: {width, height: rows * ch, channels: 4, background: '#00000000'}})
      .composite([{input: formula, left: 6, top: Math.floor((rows * ch - h) / 2)}])
      .png().toFile(file + '.tmp');
    fs.renameSync(file + '.tmp', file);
    fs.writeFileSync(meta, JSON.stringify({rows, width, height: rows * ch}));
  }
  return Array.from({length: rows}, (_, i) => `[m:${id}:${String(i).padStart(2, '0')}]`);
}
if (process.argv[2] === '--serve') {
  const net = require('node:net');
  let busy = false;
  net.createServer({allowHalfOpen: true}, socket => {
    let body = '';
    socket.on('error', () => {});
    socket.setTimeout(1000, () => socket.destroy());
    socket.on('data', chunk => {
      body += chunk;
      if (body.length > 65536) socket.destroy();
    });
    socket.on('end', async () => {
      if (busy) { socket.end('null'); return; }
      busy = true;
      try {
        const request = JSON.parse(body);
        socket.end(JSON.stringify(await render(request.source, request.width)));
      } catch { socket.end('null'); }
      finally { busy = false; }
    });
  }).listen(path.join(process.env.CODEX_LATEX_CACHE, 'renderer.sock'));
} else {
  render(fs.readFileSync(0, 'utf8'), process.argv[2])
    .then(lines => process.stdout.write(JSON.stringify(lines)))
    .catch(() => {process.exitCode = 1;});
}
