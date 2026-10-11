"""Round-trip every exported GLB through Blender and compare the asset contract.

blender -b --factory-startup --python tools/validate_models.py
"""

import json
import hashlib
from pathlib import Path

import bpy
from mathutils import Quaternion, Vector

ROOT = Path(__file__).resolve().parents[1]
MODELS = ROOT / "assets" / "models"
manifest = json.loads((MODELS / "manifest.json").read_text())
report = []

for model in manifest["models"]:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str(MODELS / model["file"]))
    bpy.context.view_layer.update()
    root = bpy.data.objects.get(model["id"] + ".Root")
    assert root is not None, f"Missing root: {model['id']}"
    assert root.location.length < 1e-5, f"Nonzero asset root: {model['id']}"
    objects = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    assert objects, f"No geometry: {model['id']}"
    points = [o.matrix_world @ Vector(c) for o in objects for c in o.bound_box]
    gltf_points = [(p.x, p.z, -p.y) for p in points]
    low = [min(p[i] for p in gltf_points) for i in range(3)]
    high = [max(p[i] for p in gltf_points) for i in range(3)]
    # Exported local-space bounds can be tighter after baked triangulation.
    # Check geometry vertex bounds against original object bounds tolerantly.
    for actual, expected in zip(low + high, model["bounds_min"] + model["bounds_max"]):
        assert abs(actual - expected) < .002, (model["id"], actual, expected)
    if model["id"].startswith("road_"):
        assert -.202 < low[1] < -.198 and 0 <= high[1] <= .021, "Road must extend below the zero-height surface"
    if model["id"] == "helicopter":
        assert abs(low[1]) < .002, "Landing skids must meet the zero-height ground plane"
    triangles = 0
    for o in objects:
        o.data.calc_loop_triangles()
        triangles += len(o.data.loop_triangles)
        assert o.data.materials, f"Missing materials: {o.name}"
    assert triangles == model["triangles"], (model["id"], triangles, model["triangles"])
    for name, expected in model["nodes"].items():
        o = bpy.data.objects.get(name)
        assert o is not None, f"Lost pivot/socket: {name}"
        assert o.parent is not None and o.parent.name == expected["parent"], f"Changed parent: {name}"
        p = o.matrix_world.translation
        for actual, value in zip((p.x, p.z, -p.y), expected["position"]):
            assert abs(actual - value) < .0001, f"Moved socket: {name}"
        axis_conversion = Quaternion((1, 0, 0), -1.5707963267948966)
        q = axis_conversion @ o.matrix_world.to_quaternion() @ axis_conversion.inverted()
        x, y, z, w = expected["rotation_xyzw"]
        expected_q = Quaternion((w, x, y, z))
        assert abs(q.dot(expected_q)) > .99999, f"Rotated socket: {name}"
    report.append({"model": model["id"], "sha256": hashlib.sha256((MODELS / model["file"]).read_bytes()).hexdigest(), "triangles": triangles,
                   "nodes_checked": len(model["nodes"]), "round_trip": "passed"})

output = ROOT / "art" / "validation"
output.mkdir(exist_ok=True)
(output / "blender_roundtrip.json").write_text(json.dumps(report, indent=2) + "\n")
print("RIVERSIDE_ROUNDTRIP_OK", len(report))
