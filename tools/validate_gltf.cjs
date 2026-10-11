// Use the official Khronos npm validator without adding a JS toolchain to the game.
// NODE_PATH=/path/to/temporary/node_modules node tools/validate_gltf.cjs
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const validator = require('gltf-validator');
const root = path.resolve(__dirname, '..');
const directory = path.join(root, 'assets', 'models');
const manifest = JSON.parse(fs.readFileSync(path.join(directory, 'manifest.json'), 'utf8'));

(async () => {
  const summaries = [];
  for (const model of manifest.models) {
    const file = path.join(directory, model.file);
    const bytes = fs.readFileSync(file);
    const report = await validator.validateBytes(new Uint8Array(bytes), {
      uri: model.file, maxIssues: 1000,
    });
    summaries.push({file: model.file, sha256: crypto.createHash('sha256').update(bytes).digest('hex'), errors: report.issues.numErrors,
      warnings: report.issues.numWarnings, messages: report.issues.messages});
    console.log(`${model.file}: ${report.issues.numErrors} errors, ${report.issues.numWarnings} warnings`);
  }
  const out = path.join(root, 'art', 'validation');
  fs.mkdirSync(out, {recursive: true});
  fs.writeFileSync(path.join(out, 'gltf_validation.json'), JSON.stringify({
    validator: validator.version(), models: summaries,
  }, null, 2) + '\n');
  if (summaries.some(s => s.errors || s.warnings)) process.exitCode = 1;
})().catch(error => { console.error(error); process.exitCode = 1; });
