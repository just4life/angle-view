"""Dev tool: convert rembg's BiRefNet ONNX export into a compact safetensors file
that the pure-Rust runtime loads. Folds BatchNorm into the deformable convs and
stores weights as f16 (default) or f32."""
import sys, collections
import numpy as np, onnx
from onnx import numpy_helper
from safetensors.numpy import save_file

src, dst = sys.argv[1], sys.argv[2]
dtype = np.float16 if (len(sys.argv) < 4 or sys.argv[3] == "f16") else np.float32
m = onnx.load(src)
inits = {i.name: numpy_helper.to_array(i) for i in m.graph.initializer}
consumers = collections.defaultdict(list)
for n in m.graph.node:
    for idx, i in enumerate(n.input):
        if i in inits:
            consumers[i].append((n, idx))

def module_path(node_name):
    segs = [s for s in node_name.split("/") if s][:-1]
    out = []
    for s in segs:
        if out and s.startswith(out[-1] + "."):
            out[-1] = s
        else:
            out.append(s)
    # torch appends _1 to scopes that are invoked a second time (the half-res backbone pass)
    out = [s[:-2] if s.endswith("_1") else s for s in out]
    return ".".join(out)

W = {}
for name, arr in inits.items():
    if not name.startswith("onnx::"):
        W[name] = arr
        continue
    n, idx = next(((n, i) for n, i in consumers[name] if n.op_type != "Identity"), (None, None))
    if n is None:
        # weight reached only through an Identity node
        ident = consumers[name][0][0]
        n, idx = next((nn, i) for nn in m.graph.node for i, inp in enumerate(nn.input) if inp == ident.output[0])
    p = module_path(n.name)
    if n.op_type == "Conv":
        W[p + (".weight" if idx == 1 else ".bias")] = arr
    elif n.op_type == "MatMul":
        W[p + ".weight_t"] = arr
    elif n.op_type == "Add" and arr.ndim == 4:
        W[p + ".rel_bias"] = arr[0]
    else:
        raise SystemExit(f"unmapped {name} {arr.shape} {n.op_type} {n.name}")

# fold BN of the deformable ASPP branches into regular_conv
for k in [k for k in W if k.endswith(".bn.running_mean")]:
    base = k[: -len(".bn.running_mean")]
    g, b, mu, var = (W.pop(f"{base}.bn.{s}") for s in ("weight", "bias", "running_mean", "running_var"))
    s = g / np.sqrt(var + 1e-5)
    w = W[f"{base}.atrous_conv.regular_conv.weight"]
    W[f"{base}.atrous_conv.regular_conv.weight"] = w * s[:, None, None, None]
    W[f"{base}.atrous_conv.regular_conv.bias"] = b - mu * s

out = {k: np.ascontiguousarray(v.astype(dtype)) for k, v in W.items() if v.dtype == np.float32}
skipped = [k for k, v in W.items() if v.dtype != np.float32]
save_file(out, dst, metadata={"arch": "birefnet-swin_v1_t", "source": src.split("/")[-1]})
print(len(out), "tensors", sum(v.size for v in out.values()), "params; skipped", skipped)
