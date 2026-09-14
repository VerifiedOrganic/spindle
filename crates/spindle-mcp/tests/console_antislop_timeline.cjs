// Run: node crates/spindle-mcp/tests/console_antislop_timeline.cjs
// Prove the console renders an anti_slop journal summary (ids + counts, no prose).
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const html = fs.readFileSync(path.join(__dirname, "../src/console.html"), "utf8");
const start = html.indexOf("function formatAntiSlop(");
const end = html.indexOf("function setSse(");
assert.notEqual(start, -1, "formatAntiSlop must exist");
assert.notEqual(end, -1, "appendEvent block must end before setSse");
const snippet = html.slice(start, end);
const context = vm.createContext({
  $: () => ({
    querySelector: () => null,
    innerHTML: "",
    appendChild() {},
    scrollTop: 0,
    scrollHeight: 0,
  }),
});
vm.runInContext(
  `${snippet}
   function esc(value) { return String(value); }
   this.formatAntiSlop = formatAntiSlop;
   this.appendEvent = appendEvent;`,
  context
);

const hard = context.formatAntiSlop(
  JSON.stringify({
    chapter: 1,
    origin: "host",
    anti_slop: {
      hard_count: 1,
      soft_count: 1,
      hard_ids: ["emotion_cocktail"],
      soft_ids: ["solitary_fade"],
    },
  })
);
assert.match(hard, /anti_slop hard=1 soft=1/);
assert.match(hard, /hard_ids=emotion_cocktail/);
assert.match(hard, /soft_ids=solitary_fade/);
assert.doesNotMatch(hard, /mix of/);
assert.doesNotMatch(hard, /excerpt/);

const missing = context.formatAntiSlop(JSON.stringify({ chapter: 1, origin: "agent" }));
assert.equal(missing, "");

const empty = context.formatAntiSlop(
  JSON.stringify({ anti_slop: { hard_count: 0, soft_count: 0, hard_ids: [], soft_ids: [] } })
);
assert.match(empty, /hard=0 soft=0/);
assert.match(empty, /hard_ids=none/);
assert.match(empty, /soft_ids=none/);

console.log("Console anti_slop timeline summary (ids + counts, no prose) passes.");
