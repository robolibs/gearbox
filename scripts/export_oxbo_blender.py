import bpy
import json
import math
import os
import sys
import traceback
from pathlib import Path
from mathutils import Matrix, Vector
from pxr import Gf, Sdf, Usd, UsdGeom, UsdPhysics

WORK=Path('/home/bresilla/oxbo_epd540e_work')
OUT=WORK/'usd'
SOURCE=WORK/'usd-backup/oxbo.original.usd'


def parent_keep(obj,parent):
    bpy.context.view_layer.update()
    world=obj.matrix_world.copy()
    obj.parent=parent;obj.matrix_parent_inverse=Matrix.Identity(4)
    obj.matrix_basis=parent.matrix_world.inverted()@world
    bpy.context.view_layer.update()


def empty(name,position,parent=None):
    obj=bpy.data.objects.new(name,None)
    bpy.context.scene.collection.objects.link(obj)
    obj.location=position
    if parent:parent_keep(obj,parent)
    bpy.context.view_layer.update()
    return obj


def attr(prim,name,type_name,value):
    prim.CreateAttribute(name,type_name,custom=True).Set(value)


def cube(stage,path,center,size):
    shape=UsdGeom.Mesh.Define(stage,path)
    points=[Gf.Vec3f(*(center[k]+sign[k]*size[k]/2 for k in range(3))) for sign in [(-1,-1,-1),(1,-1,-1),(1,1,-1),(-1,1,-1),(-1,-1,1),(1,-1,1),(1,1,1),(-1,1,1)]]
    faces=[(0,2,1),(0,3,2),(4,5,6),(4,6,7),(0,1,5),(0,5,4),(1,2,6),(1,6,5),(2,3,7),(2,7,6),(3,0,4),(3,4,7)]
    shape.CreatePointsAttr(points);shape.CreateFaceVertexCountsAttr([3]*12);shape.CreateFaceVertexIndicesAttr([i for face in faces for i in face])
    shape.CreateSubdivisionSchemeAttr('none')
    shape.CreateVisibilityAttr(UsdGeom.Tokens.invisible)
    UsdPhysics.CollisionAPI.Apply(shape.GetPrim())
    UsdPhysics.MeshCollisionAPI.Apply(shape.GetPrim()).CreateApproximationAttr('convexHull')


def main():
    OUT.mkdir(exist_ok=True)
    scene=bpy.context.scene
    c=bpy.data.objects['EPD540e | CONTROLS']
    assert not c.animation_data.action,'Export requires manual rest pose'
    assert all(c[k]==0 for k in ['steering','wheel_travel_m','header_lift','unload_deploy','harvest_enabled','unload_enabled','cab_door','header_service_hood','engine_service_doors','engine_top_covers'])
    source_blend=bpy.data.filepath
    bpy.context.view_layer.update()
    world_before={obj:obj.matrix_world.copy() for obj in scene.objects}
    for obj in scene.objects:
        if obj.animation_data:obj.animation_data_clear()
    def depth(obj):
        count=0
        while obj.parent:obj=obj.parent;count+=1
        return count
    for obj in sorted(world_before,key=depth):
        obj.matrix_world=world_before[obj]
    bpy.context.view_layer.update()
    root=empty('robot',(0,0,0))
    chassis=empty('chassis',(0,1.5,1.8),root)
    master=bpy.data.objects['OXBO EPD540e | MASTER']
    for obj in list(master.children):parent_keep(obj,chassis)
    wheels={}
    steer_objects=[]
    for axle in ['front','middle','rear']:
        for side in ['left','right']:
            key=axle+'_'+side
            spin=bpy.data.objects['JOINT | roll_'+key]
            radius=float(spin['rolling_radius_m'])
            tyre=next(o for o in spin.children if o.type=='MESH' and 'tyre' in o.name)
            points=[tyre.matrix_world@Vector(p) for p in tyre.bound_box]
            width=max(p.x for p in points)-min(p.x for p in points)
            center=list(spin.matrix_world.translation)
            spin.animation_data_clear();parent_keep(spin,root);spin.name='wheel_'+key
            wheels[key]={'center':center,'radius':radius,'width':width}
            if axle!='middle':
                steer=bpy.data.objects['JOINT | steer_'+key]
                steer.animation_data_clear();parent_keep(steer,root);steer.name='steer_'+key
                steer_objects.append(steer)
    visible=[o for o in scene.objects if o.type in {'MESH','CURVE','FONT'} and o.visible_get() and not o.hide_render and '90_Presentation' not in {col.name for col in o.users_collection}]
    bpy.ops.object.select_all(action='DESELECT')
    for obj in visible:
        if obj.type in {'CURVE','FONT'}:obj.select_set(True);bpy.context.view_layer.objects.active=obj
    if bpy.context.selected_objects:bpy.ops.object.convert(target='MESH')
    selected=set(visible)|set(bpy.context.selected_objects)|set(steer_objects)|{root,chassis}
    for obj in list(selected):
        while obj.parent:
            obj=obj.parent;selected.add(obj)
    bpy.ops.object.select_all(action='DESELECT')
    for obj in selected:
        if obj.name in scene.objects:obj.select_set(True)
    bpy.context.view_layer.objects.active=root
    for obj in visible:
        if obj in world_before:
            error=max(abs(obj.matrix_world[i][j]-world_before[obj][i][j]) for i in range(4) for j in range(4))
            assert error<2e-5,(obj.name,'Export parenting moved geometry',error)
    bpy.ops.wm.usd_export(filepath=str(OUT/'oxbo_geom.usdc'),selected_objects_only=True,root_prim_path='',export_animation=False,export_armatures=False,export_shapekeys=False,export_hair=False,export_lights=False,export_cameras=False,convert_world_material=False,export_custom_properties=False,allow_unicode=False,triangulate_meshes=True,generate_preview_surface=True,export_materials=True,export_textures_mode='NEW',relative_paths=True,overwrite_textures=True)
    geometry=Usd.Stage.Open(str(OUT/'oxbo_geom.usdc'))
    if geometry.GetPrimAtPath('/_materials'):
        editor=Usd.NamespaceEditor(geometry)
        editor.MovePrimAtPath('/_materials','/robot/Materials')
        assert editor.ApplyEdits(),'Material relocation failed'
        geometry.GetRootLayer().Save()
    stage=Usd.Stage.Open(Sdf.Layer.CreateNew(str(OUT/'oxbo.usd'),args={'format':'usda'}))
    stage.GetRootLayer().subLayerPaths=['oxbo_geom.usdc']
    original=Sdf.Layer.FindOrOpen(str(SOURCE))
    robot=stage.GetPrimAtPath('/robot')
    assert robot,'Blender export did not author /robot'
    stage.SetDefaultPrim(robot)
    UsdGeom.SetStageUpAxis(stage,UsdGeom.Tokens.z)
    UsdGeom.SetStageMetersPerUnit(stage,1)
    UsdPhysics.SetStageKilogramsPerUnit(stage,1)
    robot.SetMetadata('apiSchemas',Sdf.TokenListOp.CreateExplicit(['GearboxMachineAPI','GearboxControllerAPI:drive']))
    for prop in original.GetPrimAtPath('/robot').properties:
        if prop.name.startswith('gearbox:'):
            Sdf.CopySpec(original,prop.path,stage.GetRootLayer(),prop.path)
    for path in ['/robot/Joints','/PhysicsScene']:
        Sdf.CopySpec(original,path,stage.GetRootLayer(),path)
    for key,value in {'wheelBase':4.86,'wheelRadius':wheels['rear_left']['radius'],'trackWidth':2.4,'frontTrackWidth':2.43,'rearTrackWidth':2.4}.items():
        robot.GetAttribute('gearbox:controller:drive:'+key).Set(value)
    robot.SetDisplayName('Oxbo EPD540e - six wheel')
    attr(robot,'gearbox:asset:model',Sdf.ValueTypeNames.String,'EPD540e')
    attr(robot,'gearbox:asset:source',Sdf.ValueTypeNames.String,source_blend)
    lift=max(w['radius']-w['center'][2] for w in wheels.values())
    transform=UsdGeom.Xformable(robot)
    transform.ClearXformOpOrder();transform.AddTranslateOp().Set(Gf.Vec3d(0,0,lift))
    bodies={'chassis':(15000,(22000,72000,76000))}
    bodies.update({'wheel_'+key:(260,(68,38,38)) for key in wheels})
    bodies.update({'steer_'+key:(60,(4,4,4)) for key in wheels if not key.startswith('middle')})
    for name,(mass,inertia) in bodies.items():
        prim=stage.GetPrimAtPath('/robot/'+name)
        assert prim,name
        UsdPhysics.RigidBodyAPI.Apply(prim)
        api=UsdPhysics.MassAPI.Apply(prim);api.CreateMassAttr(mass);api.CreateDiagonalInertiaAttr(Gf.Vec3f(*inertia))
    chassis_prim=stage.GetPrimAtPath('/robot/chassis')
    UsdPhysics.MassAPI(chassis_prim).CreateCenterOfMassAttr(Gf.Vec3f(0,-.35,-.55))
    UsdPhysics.FilteredPairsAPI.Apply(chassis_prim).CreateFilteredPairsRel().SetTargets([Sdf.Path('/robot/wheel_'+key) for key in wheels])
    cube(stage,'/robot/chassis/collision_main',(0,0,0),(1.3,6.4,1.4))
    cube(stage,'/robot/chassis/collision_upper',(0,0,.95),(2.85,7.5,1.1))
    cube(stage,'/robot/chassis/collision_header',(-.15,-5.45,-1.0),(3.5,1.2,.6))
    for key,wheel in wheels.items():
        cylinder=UsdGeom.Cylinder.Define(stage,'/robot/wheel_'+key+'/tire_collider')
        cylinder.CreateAxisAttr('X');cylinder.CreateRadiusAttr(wheel['radius']);cylinder.CreateHeightAttr(wheel['width'])
        cylinder.CreateVisibilityAttr(UsdGeom.Tokens.invisible)
        UsdPhysics.CollisionAPI.Apply(cylinder.GetPrim())
    cache=UsdGeom.XformCache()
    for prim in stage.GetPrimAtPath('/robot/Joints').GetChildren():
        joint=UsdPhysics.Joint(prim)
        key=prim.GetName().removeprefix('steer_').removeprefix('rev_')
        center=Gf.Vec3d(*wheels[key]['center'])+Gf.Vec3d(0,0,lift)
        for index in [0,1]:
            body=stage.GetPrimAtPath(prim.GetRelationship('physics:body'+str(index)).GetTargets()[0])
            local=cache.GetLocalToWorldTransform(body).GetInverse().Transform(center)
            prim.GetAttribute('physics:localPos'+str(index)).Set(Gf.Vec3f(*local))
        joint.CreateCollisionEnabledAttr(False)
    mesh_count=sum(p.IsA(UsdGeom.Mesh) for p in stage.Traverse())
    stage.GetRootLayer().customLayerData={'creator':'Detailed EPD540e from articulated Blender model','visualMeshCount':mesh_count,'groundLiftMeters':lift,'mechanismState':'stowed; Blender control scripts are not a Gearbox controller'}
    (OUT/'oxbo.usd').write_text(stage.GetRootLayer().ExportToString().rstrip()+'\n')
    report={'source':source_blend,'output':str(OUT/'oxbo.usd'),'meshes':mesh_count,'wheel_data':wheels,'ground_lift_m':lift,'rigid_bodies':len(bodies),'joints':10}
    (OUT/'export-report.json').write_text(json.dumps(report,indent=2))
    print(json.dumps(report,indent=2),flush=True)


try:
    main()
except Exception:
    traceback.print_exc();sys.stdout.flush();sys.stderr.flush();os._exit(1)
sys.stdout.flush();sys.stderr.flush();os._exit(0)
