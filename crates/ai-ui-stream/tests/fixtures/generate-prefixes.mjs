// Run from the repository root with the pinned upstream checkout as argv[2].
// Only ordinary, safe object keys are included; this does not emulate
// secure-json-parse and does not cover its prototype-key policy.
import { pathToFileURL } from "node:url";
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
const reference = process.argv[2];
if (!reference) throw new Error("provide the pinned upstream checkout path");
const fixJsonPath = `${reference}/packages/assistant-stream/src/utils/json/fix-json.ts`;
// Pin the actual source, not just the checkout's HEAD: the reference may be a
// sparse checkout or contain local edits. A different implementation must not
// silently overwrite the checked-in differential expectations.
const sourceHash = createHash("sha256").update(readFileSync(fixJsonPath)).digest("hex");
if (sourceHash !== "0de84fb67cd30173a6172bc9c4a36a3d6451093e87bd9197af2fffc628d24a8b") {
  throw new Error("fix-json.ts does not match the pinned assistant-ui source");
}
const { fixJson } = await import(pathToFileURL(fixJsonPath).href);
const inputs = [
  '{"text":"brace } quote \\" slash \\\\ emoji 😀","nested":{"values":[1,-2.5e3,true,false,null]}}',
  '{"escaped":"line\\nfeed","unicode":"\\u00e9"}',
  '{"duplicate":"first","duplicate":"second","tail":0}',
  '{"constructor":1,"tail":"ok"}',
  '{"2":2,"1":1,"nested":{"10":"ten","0":"zero"}}',
  '{\n  "a" : [ 1 , { "b" : true } ] ,\n  "c" : "d"\n}\n',
];
function parse(s) {
  if (!s) return {value:{},state:"Partial",partial_path:[]};
  try { const v=JSON.parse(s); if (!v || typeof v !== "object") throw 0; return {value:v,state:"Complete",partial_path:[]}; } catch {}
  try { const [fixed,partial_path]=fixJson(s); const v=JSON.parse(fixed); if (!v || typeof v!=="object") throw 0; return {value:v,state:"Partial",partial_path}; } catch { return null; }
}
const cases = inputs.map(input => {
 let prefix="", expected=parse(""); const steps=[];
 for (const c of input) { prefix+=c; expected=parse(prefix) ?? expected; steps.push({delta:c, ...expected}); }
 return {input,steps};
});
writeFileSync("crates/assistant-partial-json/tests/fixtures/upstream-prefixes.json", JSON.stringify(cases));
