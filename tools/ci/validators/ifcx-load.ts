// What `ifcx-cli compose --no-fetch <imports...> <file> <out>` does, minus writing the composed tree: the CLI pretty-prints it
// with JSON.stringify, which throws "RangeError: Invalid string length" past Node's string limit on large models.
// Same buildingSMART functions, same validation (LoadIfcxFile with validate=true).
import * as fs from "fs";
import { LoadIfcxFile } from "../ifcx-core/workflows";
import { InMemoryLayerProvider, StackedLayerProvider } from "../ifcx-core/layers/layer-providers";
import { IfcxLayerStackBuilder } from "../ifcx-core/layers/layer-stack";

async function main() {
    const paths = process.argv.slice(2);
    const files = paths.map((p) => JSON.parse(fs.readFileSync(p).toString()));
    const first = files[files.length - 1]; // the model is the last path, the imports come first
    const userDefinedOrder: any = { header: { ...first.header }, imports: files.map((f: any) => ({ uri: f.header.id })), schemas: {}, data: [] };
    userDefinedOrder.header.id = "_cli";
    const provider = new StackedLayerProvider([new InMemoryLayerProvider().AddAll([userDefinedOrder, ...files])]);
    const stack = await new IfcxLayerStackBuilder(provider).FromId("_cli").Build();
    if (stack instanceof Error) throw stack;
    const composed: any = LoadIfcxFile(stack.GetFederatedLayer(), true, true);
    let n = 0;
    const walk = (node: any) => { n++; for (const c of Object.values(node.children ?? {})) walk(c); };
    walk(composed);
    console.log(`composed nodes=${n}`);
}
main().catch((e) => { console.error(`Error: ${e?.message ?? JSON.stringify(e)}`); process.exit(1); });
