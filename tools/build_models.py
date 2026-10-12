"""Rebuild original Riverside Escape models with Blender's bundled Python.

blender -b --factory-startup --python tools/build_models.py -- --render
Units: metres. Authoring: +Y forward / +Z up. GLB: -Z forward / +Y up.
"""

import argparse
import json
import math
import sys
from pathlib import Path

import bpy
from mathutils import Quaternion, Vector

ROOT = Path(__file__).resolve().parents[1]
MODELS = ROOT / "assets" / "models"
ART = ROOT / "art"
PREVIEWS = ART / "previews"
ASSETS = {}
CURRENT = None
PREFIX = ""
RENDER_ONLY = set()


def material(name, color, metallic=0.0, roughness=0.65):
    m = bpy.data.materials.new(name)
    m.diffuse_color = (*color, 1)
    m.use_nodes = True
    bsdf = m.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = (*color, 1)
    bsdf.inputs["Metallic"].default_value = metallic
    bsdf.inputs["Roughness"].default_value = roughness
    return m


def attach(obj, name, mat=None, parent=None):
    obj.name = PREFIX + name
    for col in list(obj.users_collection):
        col.objects.unlink(obj)
    CURRENT.objects.link(obj)
    obj.parent = parent
    if mat:
        obj.data.materials.append(M[mat])
    return obj


def empty(name, pos=(0, 0, 0), parent=None, socket=False):
    o = bpy.data.objects.new(PREFIX + name, None)
    CURRENT.objects.link(o)
    o.location = pos
    o.parent = parent
    o.empty_display_type = "ARROWS" if socket else "PLAIN_AXES"
    o.empty_display_size = 0.12
    if socket:
        o["socket"] = name
    return o


def cube(name, pos, scale, mat, parent=None, bevel=0):
    bpy.ops.mesh.primitive_cube_add(size=1, location=pos)
    o = attach(bpy.context.object, name, mat, parent)
    o.scale = scale
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    if bevel:
        mod = o.modifiers.new("Small edge facets", "BEVEL")
        mod.width = bevel
        mod.segments = 1
        bpy.ops.object.modifier_apply(modifier=mod.name)
    return o


def sphere(name, pos, scale, mat, parent=None, segments=12, rings=6):
    bpy.ops.mesh.primitive_uv_sphere_add(segments=segments, ring_count=rings, radius=1, location=pos)
    o = attach(bpy.context.object, name, mat, parent)
    o.scale = scale
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    return o


def tube(name, a, b, radius, mat, parent=None, vertices=10, radius2=None):
    a, b = Vector(a), Vector(b)
    bpy.ops.mesh.primitive_cone_add(vertices=vertices, radius1=radius,
                                  radius2=radius if radius2 is None else radius2,
                                  depth=(b - a).length, location=(a + b) / 2)
    o = attach(bpy.context.object, name, mat, parent)
    o.rotation_euler = (b - a).to_track_quat("Z", "Y").to_euler()
    return o


def torus(name, pos, major, minor, mat, parent=None, axis="X"):
    bpy.ops.mesh.primitive_torus_add(major_radius=major, minor_radius=minor,
                                   major_segments=32, minor_segments=6, location=pos)
    o = attach(bpy.context.object, name, mat, parent)
    if axis == "X":
        o.rotation_euler.y = math.pi / 2
    elif axis == "Y":
        o.rotation_euler.x = math.pi / 2
    return o


def mesh(name, verts, faces, mat, parent=None):
    data = bpy.data.meshes.new(PREFIX + name)
    data.from_pydata(verts, [], faces)
    data.update()
    o = bpy.data.objects.new(PREFIX + name, data)
    CURRENT.objects.link(o)
    o.parent = parent
    data.materials.append(M[mat])
    # Recalculate outward normals for procedural solids.
    bpy.ops.object.select_all(action="DESELECT")
    o.select_set(True)
    bpy.context.view_layer.objects.active = o
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.mesh.normals_make_consistent(inside=False)
    bpy.ops.object.mode_set(mode="OBJECT")
    return o


def hull(name, rings, mat, parent, sides=8):
    # Rings: (forward Y, centre Z, half width, half height).
    verts = [(w * math.cos(i * 2 * math.pi / sides), y,
              z + h * math.sin(i * 2 * math.pi / sides))
             for y, z, w, h in rings for i in range(sides)]
    faces = [tuple(range(sides - 1, -1, -1))]
    for r in range(len(rings) - 1):
        for i in range(sides):
            j = (i + 1) % sides
            faces.append((r * sides + i, r * sides + j,
                          (r + 1) * sides + j, (r + 1) * sides + i))
    faces.append(tuple((len(rings) - 1) * sides + i for i in range(sides)))
    return mesh(name, verts, faces, mat, parent)


def begin(name):
    global CURRENT, PREFIX
    CURRENT = bpy.data.collections.new(name)
    bpy.context.scene.collection.children.link(CURRENT)
    PREFIX = name + "."
    root = empty("Root")
    root["asset_id"] = name
    root["units"] = "metres"
    ASSETS[name] = (CURRENT, root)
    return root


def consolidate(collection):
    # Join only meshes sharing a parent AND material. Pivots/sockets survive,
    # while a static prop costs only one glTF primitive per material.
    groups = {}
    for o in list(collection.objects):
        if o.type == "MESH":
            key = (o.parent, o.data.materials[0].name)
            groups.setdefault(key, []).append(o)
    for (_, mat), objects in groups.items():
        bpy.ops.object.select_all(action="DESELECT")
        for o in objects:
            o.select_set(True)
        bpy.context.view_layer.objects.active = objects[0]
        if len(objects) > 1:
            bpy.ops.object.join()
        bpy.context.object.name = PREFIX + objects[0].parent.name.split(".")[-1] + "_" + mat


def bicycle():
    root = begin("bicycle")
    rear, front, crank = (0, -1.08, 0.35), (0, 0.95, 0.35), (0, -0.10, 0.35)
    saddle, neck = (0, -0.23, 0.91), (0, 0.68, 0.96)
    for a, b in [(rear, saddle), (rear, crank), (crank, saddle), (crank, neck), (saddle, neck)]:
        tube("Frame", a, b, .037, "Teal", root)
    for x in [-.085, .085]:
        tube("Longtail stays", (x, -1.08, .35), (0, -.23, .91), .026, "Teal", root)
        tube("Cargo rack stay", (x, -1.08, .38), (x, -1.12, .99), .022, "Steel", root)
    steering = empty("Steering", neck, root)
    for x in [-.075, .075]:
        tube("Fork", (x, 0, 0), (x, .27, -.61), .028, "Orange", steering)
    tube("Handle stem", (0, 0, 0), (0, -.07, .22), .026, "Steel", steering)
    tube("Handlebar", (-.34, -.07, .22), (.34, -.07, .22), .023, "Steel", steering)
    for x in [-.29, .29]:
        tube("Grip", (x, -.065, .22), (x, -.23, .22), .031, "Rubber", steering)
    for label, parent, centre in [("WheelRear", root, rear), ("WheelFront", steering, (0, .27, -.61))]:
        pivot = empty(label, centre, parent)
        torus("Tire", (0, 0, 0), .302, .047, "Rubber", pivot)
        torus("Rim", (0, 0, 0), .265, .013, "Steel", pivot)
        tube("Hub", (-.075, 0, 0), (.075, 0, 0), .039, "Steel", pivot)
        for i in range(12):
            a = i * math.tau / 12
            tube("Spoke", (0, 0, 0), (0, .258 * math.cos(a), .258 * math.sin(a)), .006, "Steel", pivot, 6)
        cube("Wheel reflector", (.025, .21, 0), (.022, .07, .032), "Cream", pivot)
    tube("Seat post", saddle, (0, -.23, 1.02), .026, "Steel", root)
    cube("Rider saddle", (0, -.23, 1.045), (.28, .36, .07), "Rubber", root, .04)
    cube("Passenger rack", (0, -.94, .995), (.38, .65, .07), "Steel", root, .02)
    cube("Passenger cushion", (0, -.94, 1.07), (.34, .58, .10), "Orange", root, .04)
    for x in [-.20, .20]:
        tube("Foot peg", (x, -.92, .34), (x * 1.7, -.92, .34), .024, "Steel", root)
    pedal = empty("Crank", crank, root)
    torus("Chainring", (.10, 0, 0), .13, .014, "Steel", pedal)
    for x, z in [(.13, -.15), (-.13, .15)]:
        tube("Crank arm", (x, 0, 0), (x, 0, z), .020, "Steel", pedal)
        platform = empty("Pedal_R" if x > 0 else "Pedal_L", (x * 1.65, 0, z), pedal)
        cube("Pedal", (0, 0, 0), (.18, .11, .035), "Rubber", platform, .01)
    for z in [-.11, .11]:
        tube("Chain", (.105, -1.08, .35 + z / 2), (.105, -.10, .35 + z), .01, "Dark", root, 6)
    sphere("Headlight", (0, .735, 1.0), (.065, .055, .06), "Cream", root)
    cube("Tail lamp", (0, -1.27, 1.02), (.13, .035, .045), "Red", root)
    empty("Seat_Rider", (0, -.23, 1.08), root, True)
    empty("Seat_Gunner", (0, -.94, 1.12), root, True)
    empty("Camera_Rider", (0, -.23, 1.80), root, True)
    root["wheel_radius_m"] = .349
    root["wheelbase_m"] = 2.03
    root["collision_hint"] = "Box centre (0,1.0,0), size (0.8,2.0,2.8) in glTF axes; tune in simulation"


def character(kind):
    root = begin(kind)
    coat = "Teal" if kind == "rider" else "Orange"
    cube("Pelvis", (0, 0, .045), (.30, .23, .18), "Dark", root, .035)
    torso = empty("Torso", (0, 0, .13), root)
    sphere("Jacket", (0, .055, .20), (.205, .145, .28), coat, torso)
    cube("Jacket zipper", (0, .191, .22), (.013, .008, .34), "Cream", torso)
    cube("Reflective chest", (0, .184, .31), (.30, .018, .035), "Cream", torso)
    cube("Backpack", (0, -.115, .20), (.29, .15, .32), "Orange" if kind == "rider" else "Teal", torso, .04)
    for x in [-.115, .115]:
        points = [(x, .11, .01), (x, .18, .14), (x, .17, .30), (x, .10, .43)]
        for a, b in zip(points, points[1:]):
            tube("Backpack strap", a, b, .012, "Dark", torso, 6)
    head = empty("Head", (0, .04, .48), torso)
    tube("Neck", (0, 0, -.07), (0, 0, .04), .057, "Skin", head)
    sphere("Face", (0, .025, .095), (.118, .112, .145), "Skin", head)
    sphere("Nose", (0, .137, .086), (.026, .027, .024), "Skin", head, 8, 4)
    sphere("Helmet", (0, .015, .19), (.143, .135, .105), "Cream" if kind == "rider" else "Orange", head)
    cube("Helmet brim", (0, .117, .18), (.26, .10, .024), "Teal" if kind == "rider" else "Dark", head, .01)
    for x in [-.065, 0, .065]:
        cube("Helmet vent", (x, .035, .285), (.021, .075, .01), "Dark", head, .006)
    for x in [-.045, .045]:
        sphere("Eye", (x, .126, .107), (.012, .009, .012), "Dark", head, 8, 4)
    for side in [-1, 1]:
        x = side * .17
        shoulder = empty("Shoulder_" + ("L" if side < 0 else "R"), (x, .07, .34), torso)
        if kind == "rider":
            elbow_pos = (side * .055, .24, -.18)
            hand_pos = (side * .12, .61, -.37)
        elif side > 0:
            elbow_pos, hand_pos = (.08, .15, -.22), (.085, .27, -.06)
        else:
            elbow_pos, hand_pos = (.15, .22, -.15), (.425, .44, .015)
        tube("Sleeve", (0, 0, 0), elbow_pos, .065, coat, shoulder, radius2=.055)
        elbow = empty("Elbow_" + str(side), elbow_pos, shoulder)
        target = Vector(hand_pos) - Vector(elbow_pos)
        tube("Forearm", (0, 0, 0), target, .048, "Skin", elbow, radius2=.036)
        sphere("Glove", target, (.052, .052, .050), "Dark", elbow, 8, 4)
        hip = empty("Hip_" + str(side), (side * .13, .025, 0), root)
        if kind == "rider":
            # Canonical down-axis bones, posed once for the source preview.
            # Runtime two-bone IK keeps the feet on the moving pedal platforms.
            upper, lower = .42, .43
            tube("Thigh", (0, 0, 0), (0, 0, -upper), .082, "Dark", hip, radius2=.062)
            knee = empty("Knee_" + str(side), (0, 0, -upper), hip)
            tube("Trouser leg", (0, 0, 0), (0, 0, -lower + .06), .056, "Dark", knee, radius2=.041)
            ankle = empty("Ankle_" + str(side), (0, 0, -lower), knee)
            target = Vector((side * .2145, .13, .35 - side * .15 + .07 - 1.08)) - Vector(hip.location)
            d = min(target.length, upper + lower - .001)
            axis = target.normalized()
            bend = (Vector((0, 1, 0)) - axis * axis.dot(Vector((0, 1, 0)))).normalized()
            along = (upper * upper - lower * lower + d * d) / (2 * d)
            joint = axis * along + bend * math.sqrt(max(0, upper * upper - along * along))
            q1 = Vector((0, 0, -1)).rotation_difference(joint.normalized())
            q2 = Vector((0, 0, -1)).rotation_difference((target - joint).normalized())
            hip.rotation_mode = knee.rotation_mode = ankle.rotation_mode = "QUATERNION"
            hip.rotation_quaternion = q1
            knee.rotation_quaternion = q1.inverted() @ q2
            ankle.rotation_quaternion = q2.inverted()
        else:
            knee_pos = (side * .045, .29, -.25)
            foot_pos = (side * .055, -.02, -.72)
            tube("Thigh", (0, 0, 0), knee_pos, .088, "Dark", hip, radius2=.068)
            knee = empty("Knee_" + str(side), knee_pos, hip)
            target = Vector(foot_pos) - Vector(knee_pos)
            tube("Trouser leg", (0, 0, 0), target + Vector((0, 0, .05)), .060, "Dark", knee, radius2=.043)
            ankle = empty("Ankle_" + str(side), target, knee)
        # Exposed toes, white footbed, blue sole and white Y straps.
        cube("Blue sandal sole", (0, .04, -.065), (.135, .26, .03), "SandalBlue", ankle, .022)
        cube("White footbed", (0, .04, -.046), (.127, .252, .014), "SandalWhite", ankle, .02)
        sphere("Bare foot", (0, .012, -.013), (.052, .089, .032), "Skin", ankle)
        for toe in range(5):
            sphere("Toe", (-.041 + toe * .020, .098 + .011 * (1 - toe / 4), -.019),
                   (.011, .024 - toe * .002, .014), "Skin", ankle, 8, 4)
        for side_x in [-1, 1]:
            tube("White sandal strap", (0, .081, .005), (side_x * .057, -.031, .010), .013, "SandalWhite", ankle, 8)
        tube("Blue toe post", (0, .081, -.035), (0, .081, .009), .009, "SandalBlue", ankle, 8)
    empty("Camera_Aim", (0, .09, .72), root, True)
    if kind == "gunner":
        empty("WeaponMount", (.255, .30, .43), torso, True)
    root["origin"] = "Seated pelvis; attach root at matching bicycle seat socket"
    root["rig"] = "Rigid segment pivots; no skinning or baked animation clips"


def launcher():
    root = begin("launcher")
    tube("Tube", (0, -.34, 0), (0, .37, 0), .085, "Teal", root, 12)
    for y in [-.30, .30]:
        torus("Reinforced collar", (0, y, 0), .085, .018, "Dark", root, "Y")
    tube("Dark bore", (0, .371, 0), (0, .377, 0), .064, "Dark", root, 12)
    # Open sight: opaque geometry must not block the ADS centerline.
    for x in [-.052, .052]:
        cube("Sight upright", (x, .05, .15), (.012, .065, .10), "Orange", root, .003)
    for z in [.10, .20]:
        cube("Sight crossbar", (0, .05, z), (.116, .065, .012), "Orange", root, .003)
    cube("Sight mount", (0, .05, .087), (.035, .065, .028), "Dark", root)
    cube("Grip", (0, .04, -.12), (.065, .10, .17), "Dark", root, .012)
    cube("Rear pad", (0, -.19, -.10), (.12, .16, .08), "Dark", root, .012)
    empty("Muzzle", (0, .40, 0), root, True)
    empty("Grip", (0, .04, -.18), root, True)


def shotgun():
    root = begin("shotgun")
    # Fictional pump shotgun; +Y muzzle direction becomes glTF -Z.
    cube("Receiver", (0, -.06, 0), (.085, .30, .105), "Navy", root, .012)
    cube("Stock", (0, -.36, -.025), (.075, .32, .13), "Wood", root, .025)
    cube("Recoil pad", (0, -.53, -.026), (.08, .025, .14), "Rubber", root, .008)
    tube("Barrel", (0, .06, .026), (0, .63, .026), .026, "Steel", root, 12)
    tube("Muzzle bore", (0, .632, .026), (0, .636, .026), .019, "Dark", root, 12)
    tube("Magazine tube", (0, .07, -.035), (0, .48, -.035), .023, "Navy", root, 10)
    pump = empty("Pump", (0, .25, -.029), root)
    cube("Pump grip", (0, 0, 0), (.095, .23, .082), "Wood", pump, .012)
    for y in [-.075, -.04, -.005, .03, .065]:
        cube("Pump rib", (0, y, 0), (.100, .011, .086), "Dark", pump, .003)
    cube("Pistol grip", (0, -.135, -.092), (.066, .088, .12), "Wood", root, .014)
    tube("Trigger guard", (-.025, -.085, -.045), (-.025, -.015, -.102), .007, "Steel", root, 8)
    tube("Trigger guard", (-.025, -.015, -.102), (-.025, -.12, -.12), .007, "Steel", root, 8)
    cube("Ejection port", (.044, -.024, .005), (.004, .105, .046), "Dark", root)
    for x in [-.022, .022]:
        cube("Rear sight", (x, -.16, .077), (.009, .025, .035), "Dark", root)
    cube("Front sight", (0, .54, .075), (.008, .021, .027), "SandalWhite", root)
    empty("Muzzle", (0, .65, .026), root, True)
    empty("Grip", (0, -.135, -.12), root, True)


def projectile():
    root = begin("missile")
    tube("Body", (0, -.22, 0), (0, .20, 0), .055, "Cream", root, 10)
    tube("Nose", (0, .20, 0), (0, .37, 0), .055, "Orange", root, 10, .001)
    tube("Nozzle", (0, -.25, 0), (0, -.22, 0), .04, "Dark", root, 10)
    for i in range(4):
        fin = mesh("Fin", [(0, -.22, 0), (.15, -.22, 0), (.05, -.02, 0), (0, -.02, 0)], [(0, 1, 2, 3)], "Teal", root)
        solid = fin.modifiers.new("Fin thickness", "SOLIDIFY")
        solid.thickness = .009
        bpy.context.view_layer.objects.active = fin
        bpy.ops.object.modifier_apply(modifier=solid.name)
        fin.rotation_euler.y = i * math.pi / 2
    empty("Exhaust", (0, -.27, 0), root, True)
    root["purpose"] = "Stylized game projectile; no flight or damage logic"


def helicopter():
    root = begin("helicopter")
    body = hull("Fuselage", [(-1.45, 1.35, .32, .37), (-.85, 1.35, .70, .68),
                (.20, 1.35, .73, .70), (.90, 1.30, .66, .60),
                (1.55, 1.18, .52, .47), (1.93, 1.10, .17, .20)], "Navy", root)
    # Cockpit glazing shares the fuselage surface: no intersecting glass shell.
    body.data.materials.append(M["Glass"])
    for ring in range(2, 5):
        for side in range(4):
            body.data.polygons[1 + ring * 8 + side].material_index = 1
    for side in [-1, 1]:
        points = [(side * .52, .20, 1.85), (side * .47, .90, 1.73),
                  (side * .372, 1.55, 1.515), (side * .125, 1.93, 1.245)]
        for a, b in zip(points, points[1:]):
            tube("Cockpit frame", a, b, .022, "Navy", root)
        cube("Door panel", (side * .685, -.45, 1.32), (.025, .80, .80), "Teal", root, .09)
        cube("Side window", (side * .708, -.39, 1.58), (.025, .58, .27), "Glass", root, .035)
        cube("Door handle", (side * .732, -.61, 1.26), (.025, .14, .025), "Cream", root)
        tube("Skid", (side * .90, -1.15, .19), (side * .90, 1.40, .19), .065, "Steel", root)
        tube("Skid nose", (side * .90, 1.40, .19), (side * .90, 1.67, .38), .065, "Steel", root)
        for y in [-.68, .75]:
            tube("Landing strut", (side * .43, y, .80), (side * .90, y, .20), .050, "Navy", root)
        tube("Stub wing", (side * .55, -.10, 1.0), (side * 1.10, -.10, 1.0), .09, "Navy", root)
        tube("Rocket pod", (side * 1.10, -.47, .96), (side * 1.10, .31, .96), .17, "Orange", root, 8)
        for dx, dz in [(-.058, -.058), (-.058, .058), (.058, -.058), (.058, .058)]:
            tube("Pod port", (side * 1.10 + dx, .315, .96 + dz),
                 (side * 1.10 + dx, .326, .96 + dz), .035, "Dark", root, 8)
        empty("RocketSocket_" + str(side), (side * 1.10, .36, .96), root, True)
    hull("Tail boom", [(-1.20, 1.45, .32, .32), (-3.75, 1.77, .09, .14)], "Teal", root)
    hull("Tail band", [(-3.14, 1.69, .154, .193), (-3.40, 1.72, .13, .18)], "Orange", root)
    fin = mesh("Tail fin", [(-.045, -3.95, 1.55), (-.045, -4.10, 2.73), (-.045, -3.60, 2.54), (-.045, -3.24, 1.59),
                            (.045, -3.95, 1.55), (.045, -4.10, 2.73), (.045, -3.60, 2.54), (.045, -3.24, 1.59)],
               [(0, 1, 2, 3), (4, 7, 6, 5), (0, 4, 5, 1), (1, 5, 6, 2), (2, 6, 7, 3), (3, 7, 4, 0)], "Navy", root)
    cube("Tail stabilizer", (0, -3.03, 1.61), (1.35, .35, .06), "Orange", root, .025)
    sphere("Engine cowling", (0, -.50, 1.99), (.46, .75, .28), "Teal", root)
    for x in [-.30, .30]:
        tube("Exhaust", (x, -.83, 2.03), (x, -1.23, 2.10), .10, "Dark", root)
    tube("Rotor mast", (0, -.17, 2.05), (0, -.17, 2.50), .09, "Steel", root)
    rotor = empty("MainRotor", (0, -.17, 2.51), root)
    sphere("Rotor hub", (0, 0, 0), (.22, .22, .10), "Orange", rotor)
    for i in range(4):
        blade = empty("Blade_" + str(i), parent=rotor)
        blade.rotation_euler.z = i * math.pi / 2 + .22
        cube("Blade", (1.70, 0, 0), (3.10, .18, .045), "Dark", blade, .015)
        cube("Rotor tip", (3.15, 0, .003), (.20, .19, .05), "Cream", blade)
    tail = empty("TailRotor", (.17, -3.78, 2.14), root)
    tube("Tail hub", (-.02, 0, 0), (.12, 0, 0), .07, "Steel", tail)
    for i in range(4):
        blade = empty("TailBlade_" + str(i), parent=tail)
        blade.rotation_euler.x = i * math.pi / 2 + .35
        cube("Tail blade", (.10, .29, 0), (.035, .60, .09), "Dark", blade, .009)
        cube("Tail tip", (.105, .52, 0), (.04, .14, .095), "Orange", blade)
    aim = empty("TurretYaw", (0, 1.20, .64), root)
    sphere("Turret", (0, 0, 0), (.15, .17, .14), "Dark", aim)
    pitch = empty("TurretPitch", parent=aim)
    tube("Barrel", (0, .07, 0), (0, .58, 0), .036, "Steel", pitch)
    empty("Muzzle", (0, .62, 0), pitch, True)
    empty("TargetLock", (0, .15, 1.40), root, True)
    root["animation_axes"] = "GLB MainRotor +Y; TailRotor +X; TurretYaw +Y; TurretPitch +X"
    bpy.context.view_layer.update()
    ground_z = min((o.matrix_world @ v.co).z for o in CURRENT.objects if o.type == "MESH" for v in o.data.vertices)
    for o in CURRENT.objects:
        if o.parent == root:
            o.location.z -= ground_z


def props():
    root = begin("traffic_cone")
    cube("Base", (0, 0, .035), (.40, .40, .07), "Dark", root, .035)
    for z1, z2, r1, r2, mat in [(.07, .25, .155, .102, "Orange"), (.25, .34, .102, .075, "Cream"), (.34, .53, .075, .020, "Orange")]:
        tube("Cone", (0, 0, z1), (0, 0, z2), r1, mat, root, 12, r2)

    root = begin("bollard")
    tube("Base", (0, 0, 0), (0, 0, .08), .13, "Steel", root)
    tube("Post", (0, 0, .08), (0, 0, .76), .067, "Teal", root)
    tube("Reflector", (0, 0, .59), (0, 0, .69), .071, "Cream", root)
    sphere("Cap", (0, 0, .765), (.07, .07, .055), "Orange", root)

    root = begin("concrete_barrier")
    # Jersey-style cross section, extruded sideways across X.
    profile = [(-.30, 0), (.30, 0), (.30, .16), (.13, .51), (.13, .85), (-.13, .85), (-.13, .51), (-.30, .16)]
    verts = [(x, y, z) for x in [-1.0, 1.0] for y, z in profile]
    faces = [tuple(range(7, -1, -1)), tuple(range(8, 16))] + [(i, (i + 1) % 8, (i + 1) % 8 + 8, i + 8) for i in range(8)]
    mesh("Barrier", verts, faces, "Concrete", root)
    for x in [-.76, -.38, 0, .38, .76]:
        cube("Hazard stripe", (x, .135, .67), (.18, .012, .25), "Orange", root)
    empty("Connector_L", (-1, 0, 0), root, True)
    empty("Connector_R", (1, 0, 0), root, True)

    root = begin("railing")
    for x in [-2, 0, 2]:
        tube("Post", (x, 0, 0), (x, 0, 1.10), .04, "Teal", root)
        cube("Anchor foot", (x, 0, .035), (.16, .16, .07), "Steel", root)
    for z in [.42, .98]:
        tube("Rail", (-2, 0, z), (2, 0, z), .032, "Teal", root)
    for x in [-1.5, -.75, .75, 1.5]:
        tube("Upright", (x, 0, .42), (x, 0, .98), .018, "Steel", root, 8)
    empty("Connector_L", (-2, 0, 0), root, True)
    empty("Connector_R", (2, 0, 0), root, True)

    root = begin("bridge_pier")
    cube("Footing", (0, 0, .15), (2.1, 1.7, .30), "Concrete", root, .06)
    cube("Column", (0, 0, 2.8), (1.15, 1.05, 5.30), "Concrete", root, .12)
    cube("Top cap", (0, 0, 6.30), (4.2, 1.50, .65), "Concrete", root, .08)
    for x in [-.41, 0, .41]:
        cube("Safety marker", (x, .535, .85), (.19, .022, .65), "Orange", root)

    root = begin("riverside_tree")
    tube("Trunk", (0, 0, 0), (0, 0, 2.20), .17, "Wood", root, 7, .11)
    for pos, scl, mat in [((0, 0, 2.8), (1.10, .94, 1.05), "Leaf"), ((-.65, .13, 2.30), (.75, .78, .83), "LeafLight"), ((.60, .10, 2.44), (.78, .72, .85), "LeafDark")]:
        bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1, radius=1, location=pos)
        o = attach(bpy.context.object, "Crown", mat, root)
        o.scale = scl
        bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)

    root = begin("riverside_lamp")
    tube("Foot", (0, 0, 0), (0, 0, .17), .16, "Steel", root)
    tube("Pole", (0, 0, .15), (0, 0, 3.8), .055, "Teal", root)
    tube("Lamp arm", (0, 0, 3.78), (0, .72, 3.94), .043, "Teal", root)
    cube("Lamp housing", (0, .76, 3.93), (.34, .50, .10), "Dark", root, .035)
    cube("Lamp lens", (0, .78, 3.87), (.27, .37, .022), "Cream", root)


def roads():
    root = begin("road_straight")
    # Surface is at zero so road connects without elevation offsets.
    cube("Paving", (0, 4, -.10), (4, 8, .20), "Asphalt", root)
    for x in [-1.94, 1.94]:
        cube("Edge stripe", (x, 4, .008), (.055, 8, .016), "Cream", root)
    for y in [1, 3, 5, 7]:
        cube("Centre dash", (0, y, .009), (.07, .95, .018), "Yellow", root)
    for x in [-2.15, 2.15]:
        cube("Shoulder", (x, 4, -.09), (.30, 8, .18), "Concrete", root)
    empty("Entry", (0, 0, 0), root, True)
    empty("Exit", (0, 8, 0), root, True)

    root = begin("road_curve")
    radius, angle, steps = 10, math.pi / 6, 12

    def strip(name, r0, r1, start, end, z, mat):
        verts = []
        for i in range(steps + 1):
            a = start + (end - start) * i / steps
            for r in [r0, r1]:
                verts.append((radius - r * math.cos(a), r * math.sin(a), z))
        # +Z winding is essential: Solidify must extend below the road plane.
        faces = [(i * 2, i * 2 + 2, i * 2 + 3, i * 2 + 1) for i in range(steps)]
        return mesh(name, verts, faces, mat, root)

    surface = strip("Curve paving", 8, 12, 0, angle, 0, "Asphalt")
    mod = surface.modifiers.new("Road thickness", "SOLIDIFY")
    mod.thickness = .20
    mod.offset = -1
    bpy.context.view_layer.objects.active = surface
    bpy.ops.object.modifier_apply(modifier=mod.name)
    for r in [8.06, 11.94]:
        strip("Edge stripe", r - .0275, r + .0275, 0, angle, .012, "Cream")
    for i in range(3):
        strip("Centre dash", 9.965, 10.035, (i * 2 + .5) / radius,
              min((i * 2 + 1.45) / radius, angle), .014, "Yellow")
    for r0, r1 in [(7.7, 8), (12, 12.3)]:
        strip("Shoulder", r0, r1, 0, angle, -.01, "Concrete")
    empty("Entry", (0, 0, 0), root, True)
    end = empty("Exit", (radius * (1 - math.cos(angle)), radius * math.sin(angle), 0), root, True)
    end.rotation_euler.z = -angle
    root["turn_degrees"] = 30.0
    root["centreline_radius_m"] = radius


def export_all():
    manifest = {"schema_version": 1, "units": "metres", "up": "+Y", "forward": "-Z",
                "format": "glTF 2.0 / GLB", "materials": "Opaque PBR factors; no textures or external resources",
                "source": "art/riverside_models.blend", "node_transform_space": "asset-local, glTF axes",
                "models": []}
    for name, (collection, root) in ASSETS.items():
        global PREFIX
        PREFIX = name + "."
        consolidate(collection)
        bpy.context.view_layer.update()
        bpy.ops.object.select_all(action="DESELECT")
        for o in collection.objects:
            o.select_set(True)
        bpy.context.view_layer.objects.active = root
        bpy.ops.export_scene.gltf(filepath=str(MODELS / (name + ".glb")), export_format="GLB",
                                  use_selection=True, export_yup=True, export_extras=True,
                                  export_animations=False, export_cameras=False, export_lights=False,
                                  export_apply=True)
        points = [o.matrix_world @ Vector(v) for o in collection.objects if o.type == "MESH" for v in o.bound_box]
        lo = [min(p[i] for p in points) for i in range(3)]
        hi = [max(p[i] for p in points) for i in range(3)]
        sockets = {}
        for o in collection.objects:
            if o.type == "EMPTY" and o != root:
                p = o.matrix_world.translation
                axis_conversion = Quaternion((1, 0, 0), -math.pi / 2)
                q = axis_conversion @ o.matrix_world.to_quaternion() @ axis_conversion.inverted()
                sockets[o.name] = {"position": [round(p.x, 5), round(p.z, 5), round(-p.y, 5)],
                                   "rotation_xyzw": [round(v, 6) for v in (q.x, q.y, q.z, q.w)],
                                   "parent": o.parent.name if o.parent else None,
                                   "socket": bool(o.get("socket"))}
        triangles = 0
        for o in collection.objects:
            if o.type == "MESH":
                o.data.calc_loop_triangles()
                triangles += len(o.data.loop_triangles)
        manifest["models"].append({"id": name, "file": name + ".glb", "scene": "Scene0",
            "bytes": (MODELS / (name + ".glb")).stat().st_size, "triangles": triangles,
            "mesh_objects": sum(o.type == "MESH" for o in collection.objects),
            "bounds_min": [round(lo[0], 4), round(lo[2], 4), round(-hi[1], 4)],
            "bounds_max": [round(hi[0], 4), round(hi[2], 4), round(-lo[1], 4)],
            "nodes": sockets})
    (MODELS / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def instance(asset, pos=(0, 0, 0), angle=0, scale=1, parent=None):
    # Copy objects, retain shared mesh data and the complete node hierarchy.
    col, root = ASSETS[asset]
    copies = {}
    for original in col.objects:
        copy = original.copy()
        CURRENT.objects.link(copy)
        copies[original] = copy
    for original, copy in copies.items():
        copy.parent = copies.get(original.parent)
    result = copies[root]
    result.parent = parent
    result.location = pos
    result.rotation_euler.z = angle
    result.scale = (scale,) * 3
    return result, copies


def camera(scene, pos, target, ortho):
    data = bpy.data.cameras.new("Presentation camera")
    obj = bpy.data.objects.new("Presentation camera", data)
    scene.collection.objects.link(obj)
    obj.location = pos
    obj.rotation_euler = (Vector(target) - obj.location).to_track_quat("-Z", "Y").to_euler()
    data.type = "ORTHO"
    data.ortho_scale = ortho
    scene.camera = obj


def light(scene, name, pos, energy, size, color):
    data = bpy.data.lights.new(name, "AREA")
    data.energy, data.shape, data.size, data.color = energy, "DISK", size, color
    obj = bpy.data.objects.new(name, data)
    scene.collection.objects.link(obj)
    obj.location = pos
    obj.rotation_euler = (-obj.location).to_track_quat("-Z", "Y").to_euler()


def setup_scene(name, resolution=(1400, 1050)):
    global CURRENT, PREFIX
    scene = bpy.data.scenes.new(name)
    bpy.context.window.scene = scene
    CURRENT = bpy.data.collections.new(name + " presentation")
    scene.collection.children.link(CURRENT)
    PREFIX = "Preview."
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 24
    scene.cycles.use_denoising = True
    scene.render.resolution_x, scene.render.resolution_y = resolution
    scene.render.resolution_percentage = 100
    scene.world = bpy.data.worlds.new(name + " world")
    scene.world.use_nodes = True
    scene.world.node_tree.nodes["Background"].inputs[0].default_value = (.45, .56, .57, 1)
    scene.world.node_tree.nodes["Background"].inputs[1].default_value = .45
    scene.view_settings.view_transform = "AgX"
    light(scene, "Large warm key", (3, 0, 9), 1400, 7, (1, .86, .70))
    light(scene, "Soft sky fill", (-5, 1, 5), 900, 6, (.70, .85, 1))
    light(scene, "Rim", (1, -6, 7), 1100, 5, (1, .94, .84))
    return scene


def render(scene, filename, enabled):
    scene.render.image_settings.file_format = "PNG"
    scene.render.filepath = str(PREVIEWS / filename)
    if enabled and (not RENDER_ONLY or filename in RENDER_ONLY):
        bpy.ops.render.render(write_still=True)


def previews(enabled):
    # Two close-ups prove shape/readability, then a riverside context diorama.
    scene = setup_scene("01 - Bicycle crew")
    cube("Studio floor", (0, 0, -.06), (200, 200, .10), "Backdrop")
    bike, parts = instance("bicycle")
    for role in ["rider", "gunner"]:
        seat = next(copy for orig, copy in parts.items() if orig.name == "bicycle.Seat_" + role.title())
        figure, figure_parts = instance(role, parent=seat)
        if role == "gunner":
            mount = next(copy for orig, copy in figure_parts.items() if orig.name == "gunner.WeaponMount")
            instance("launcher", parent=mount)
    camera(scene, (4.2, 5.1, 2.7), (0, -.12, .95), 3.85)
    render(scene, "bicycle_crew.png", enabled)

    scene = setup_scene("02 - Helicopter")
    cube("Studio floor", (0, 0, -.06), (200, 200, .10), "Backdrop")
    instance("helicopter")
    camera(scene, (9, 12, 7), (0, -.60, 1.35), 8.6)
    render(scene, "helicopter.png", enabled)

    scene = setup_scene("03 - Riverside kit", (1600, 1200))
    cube("Base slab", (0, 0, -.30), (12, 13, .50), "Sand", bevel=.18)
    cube("River", (-4.5, 0, -.028), (2.95, 12.8, .07), "Water", bevel=.025)
    for y in [-4, -1.2, 2.8, 4.1]:
        cube("Water glint", (-4.5, y, .011), (.80, .025, .006), "Foam")
    instance("road_straight", (0, -4, 0))
    instance("road_curve", (0, 4, 0))
    for y in [-4, 0, 4]:
        instance("railing", (-2.62, y, 0), math.pi / 2)
    for pos in [(3.4, -4, 0), (4.8, 1.1, 0), (4.1, 4.0, 0)]:
        instance("riverside_tree", pos, scale=.85)
    instance("riverside_lamp", (2.55, -2.5, 0), math.pi / 2)
    instance("bridge_pier", (3.5, 5.3, 0), scale=.8)
    instance("concrete_barrier", (.3, 2.9, .02), -.20)
    for x, y in [(-1.3, 2.4), (1.0, .6), (1.5, 1.2)]:
        instance("traffic_cone", (x, y, .02))
    for y in [-3.5, -.5, 2.5]:
        instance("bollard", (2.30, y, 0))
    bike, parts = instance("bicycle", (0, -1.9, .02), -.10)
    for role in ["rider", "gunner"]:
        seat = next(copy for orig, copy in parts.items() if orig.name == "bicycle.Seat_" + role.title())
        _, figure_parts = instance(role, parent=seat)
        if role == "gunner":
            mount = next(copy for orig, copy in figure_parts.items() if orig.name == "gunner.WeaponMount")
            instance("launcher", parent=mount)
    camera(scene, (15, 17, 14), (0, .5, 1.0), 18.5)
    render(scene, "riverside_kit.png", enabled)

    scene = setup_scene("04 - Model catalog", (2000, 1800))
    camera(scene, (8, 12, 9), (0, 0, 0), 21.5)
    q = scene.camera.rotation_euler.to_quaternion()
    right, up, toward = q @ Vector((1, 0, 0)), q @ Vector((0, 1, 0)), q @ Vector((0, 0, 1))
    # Unlit ink and paper keep typography clear regardless of studio lighting.
    for name, color in [("Catalog ink", (.006, .018, .022)), ("Catalog paper", (.76, .82, .79))]:
        mat = bpy.data.materials.new(name)
        mat.use_nodes = True
        mat.node_tree.nodes.clear()
        emission = mat.node_tree.nodes.new("ShaderNodeEmission")
        emission.inputs["Color"].default_value = (*color, 1)
        output = mat.node_tree.nodes.new("ShaderNodeOutputMaterial")
        mat.node_tree.links.new(emission.outputs[0], output.inputs["Surface"])
        M[name] = mat
    mesh("Catalog paper", [right * x + up * y - toward * 5 for x, y in [(-20, -15), (20, -15), (20, 15), (-20, 15)]],
         [(0, 1, 2, 3)], "Catalog paper")

    def label(text, x, y, size):
        data = bpy.data.curves.new(text, "FONT")
        data.body, data.align_x, data.size = text, "CENTER", size
        data.materials.append(M["Catalog ink"])
        obj = bpy.data.objects.new(text, data)
        CURRENT.objects.link(obj)
        obj.location = right * x + up * y + toward * 2
        obj.rotation_euler = scene.camera.rotation_euler

    label("R I V E R S I D E   E S C A P E", 0, 5.95, .39)
    label("MODEL KIT 01  /  LOW POLY  /  16 MODULAR ASSETS", 0, 6.30, .18)
    labels = ["BICYCLE", "RIDER", "GUNNER", "LAUNCHER", "MISSILE", "HELICOPTER",
              "TRAFFIC CONE", "BOLLARD", "CONCRETE BARRIER", "RAILING", "BRIDGE PIER",
              "RIVERSIDE TREE", "RIVERSIDE LAMP", "STRAIGHT ROAD", "30 DEGREE CURVE", "SHOTGUN"]
    for i, (name, (col, _)) in enumerate(ASSETS.items()):
        bpy.context.view_layer.update()
        points = [o.matrix_world @ Vector(p) for o in col.objects if o.type == "MESH" for p in o.bound_box]
        projected = [(p.dot(right), p.dot(up), p.dot(toward)) for p in points]
        low = Vector(tuple(min(p[j] for p in projected) for j in range(3)))
        high = Vector(tuple(max(p[j] for p in projected) for j in range(3)))
        size = min(2.45 / (high.x - low.x), 1.9 / (high.y - low.y))
        middle = (low + high) * .5
        asset_center = right * middle.x + up * middle.y + toward * middle.z
        x, y = (i % 4 - 1.5) * 4.1, 4.6 - (i // 4) * 3.3
        center = right * x + up * y
        instance(name, center - asset_center * size, scale=size)
        label(f"{i + 1:02d}  {labels[i]}", x, y - 1.40, .18)
    label("METRE SCALE  /  PBR COLOURS  /  GLB + BLENDER SOURCES", 0, -7.05, .16)
    render(scene, "model_catalog.png", enabled)


def main():
    args = argparse.ArgumentParser()
    args.add_argument("--render", action="store_true")
    args.add_argument("--render-only", nargs="*", choices=["bicycle_crew.png", "helicopter.png", "riverside_kit.png", "model_catalog.png"])
    options = args.parse_args(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
    for path in [MODELS, ART, PREVIEWS]:
        path.mkdir(parents=True, exist_ok=True)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    global M, RENDER_ONLY
    RENDER_ONLY = set(options.render_only or [])
    M = {name: material(name, rgb, metal, rough) for name, rgb, metal, rough in [
        ("Teal", (.035, .37, .32), .20, .48), ("Orange", (.92, .23, .085), .05, .55),
        ("Cream", (.92, .86, .69), 0, .65), ("Dark", (.023, .047, .060), .1, .7),
        ("Rubber", (.017, .023, .028), 0, .95), ("Steel", (.32, .44, .47), .7, .30),
        ("Navy", (.045, .095, .115), .35, .42), ("Glass", (.055, .24, .30), .48, .19),
        ("SandalBlue", (.025, .18, .70), 0, .8), ("SandalWhite", (.97, .98, 1.0), 0, .8),
        ("Skin", (.65, .36, .22), 0, .75), ("Red", (.65, .028, .018), 0, .45),
        ("Concrete", (.49, .55, .53), 0, .9), ("Wood", (.24, .14, .075), 0, .95),
        ("Leaf", (.12, .34, .22), 0, 1), ("LeafLight", (.24, .46, .27), 0, 1),
        ("LeafDark", (.065, .235, .18), 0, 1), ("Asphalt", (.12, .20, .20), 0, .95),
        ("Yellow", (.91, .61, .12), 0, .8), ("Backdrop", (.64, .72, .69), 0, .9),
        ("Sand", (.53, .60, .43), 0, .95), ("Water", (.08, .38, .42), .10, .35),
        ("Foam", (.44, .70, .68), 0, .7)]}
    library = bpy.context.scene
    library.name = "00 - Model library"
    library.unit_settings.system = "METRIC"
    bicycle()
    character("rider")
    character("gunner")
    launcher()
    projectile()
    helicopter()
    props()
    roads()
    shotgun()
    export_all()
    previews(options.render or bool(RENDER_ONLY))
    # Show each editable source in its own bay, after origin-correct exports.
    bpy.context.window.scene = library
    for i, (_, root) in enumerate(ASSETS.values()):
        root.location = ((i % 4) * 9, (i // 4) * 12, 0)
    bpy.context.window.scene = bpy.data.scenes["01 - Bicycle crew"]
    bpy.ops.object.select_all(action="DESELECT")
    # Start the .blend in the composed camera view, with useful solid colours.
    for screen in bpy.data.screens:
        for area in screen.areas:
            if area.type == "VIEW_3D":
                area.spaces.active.region_3d.view_perspective = "CAMERA"
                area.spaces.active.shading.color_type = "MATERIAL"
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(ART / "riverside_models.blend"))
    print("RIVERSIDE_MODEL_BUILD_OK", len(ASSETS))


if __name__ == "__main__":
    main()
