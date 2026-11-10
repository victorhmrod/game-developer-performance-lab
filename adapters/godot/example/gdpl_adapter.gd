extends Node3D

# Experimental Godot 4.5+ example. The measured workload is synthetic.
const ActiveNode = preload("res://active_node.gd")
const LIMITS := {"nodes": 1000, "physics": 128, "rendering": 512, "animation": 256}
const METADATA := {
    "godot_clock_us": "microseconds; Godot Time.get_ticks_usec monotonic clock since startup; not synchronized to GDPL",
    "godot_process_interval_ms": "milliseconds; last interval between adapter _process callbacks; not display time",
    "godot_fps": "frames/second; Performance.TIME_FPS, updated by Godot approximately once per second",
    "godot_process_ms": "milliseconds; Performance.TIME_PROCESS, converted from seconds",
    "godot_physics_ms": "milliseconds; Performance.TIME_PHYSICS_PROCESS, converted from seconds",
    "godot_node_count": "nodes; Performance.OBJECT_NODE_COUNT",
    "godot_workload_count": "objects; direct workload objects currently in the scene tree",
    "godot_processing_nodes": "nodes; workload nodes with processing enabled; nodes workload only",
    "godot_physics_active": "bodies; Performance.PHYSICS_3D_ACTIVE_OBJECTS; physics workload only",
    "godot_physics_pairs": "pairs; Performance.PHYSICS_3D_COLLISION_PAIRS; physics workload only",
    "godot_animation_playing": "players; workload AnimationPlayer instances reporting is_playing; animation workload only",
    "godot_animation_position_sum_s": "seconds; sum of current animation positions; animation workload only",
    "godot_render_draw_calls": "draw calls; Performance.RENDER_TOTAL_DRAW_CALLS_IN_FRAME; graphical mode only",
    "godot_render_objects": "objects; Performance.RENDER_TOTAL_OBJECTS_IN_FRAME; only when not headless",
    "godot_render_primitives": "primitives; Performance.RENDER_TOTAL_PRIMITIVES_IN_FRAME; only when not headless"
}

var reader := Thread.new()
var gate := Semaphore.new()
var state_lock := Mutex.new()
var stopping := false
var initialized := false
var negotiated := false
var last_request_id := 0
var workload_kind := ""
var maximum_load := 0
var workload_root: Node3D
var last_frame_tick := 0
var process_interval_ms := 0.0

func _ready() -> void:
    Engine.max_fps = 120
    get_tree().auto_accept_quit = false
    workload_root = Node3D.new()
    add_child(workload_root)
    var camera := Camera3D.new()
    camera.position = Vector3(0, 0, 30)
    camera.current = true
    add_child(camera)
    var error := reader.start(_read_requests)
    if error != OK:
        printerr("GDPL adapter: failed to start stdin reader: ", error)
        get_tree().quit(1)

func _process(_delta: float) -> void:
    var now := Time.get_ticks_usec()
    if last_frame_tick > 0:
        process_interval_ms = float(now - last_frame_tick) / 1000.0
    last_frame_tick = now

func _read_requests() -> void:
    var buffer := PackedByteArray()
    while true:
        var bytes := OS.read_buffer_from_stdin(1)
        if bytes.is_empty():
            call_deferred("_disconnected")
            return
        if bytes[0] == 10:
            call_deferred("_on_request", buffer.get_string_from_utf8())
            buffer.clear()
            gate.wait()
            state_lock.lock()
            var done := stopping
            state_lock.unlock()
            if done:
                return
        else:
            buffer.append(bytes[0])
            if buffer.size() > 8191:
                call_deferred("_disconnected")
                return

func _disconnected() -> void:
    _clear_workload()
    if reader.is_started():
        reader.wait_to_finish()
    get_tree().quit(1)

func _on_request(line: String) -> void:
    var receive_us := Time.get_ticks_usec()
    var parsed = JSON.parse_string(line)
    if not parsed is Dictionary:
        _reply({"version": 1, "id": 0, "ok": false, "error": "invalid JSON object"})
        gate.post()
        return
    var request: Dictionary = parsed
    var id = request.get("id", 0)
    var reply := {"version": 1, "id": id, "ok": true}
    var data = request.get("data", {})
    if not (id is float or id is int) or id <= last_request_id or id != floor(id):
        reply["ok"] = false
        reply["error"] = "request id must be a strictly increasing positive integer"
        reply["error_code"] = "invalid_request"
    elif request.get("version") != 1 or not data is Dictionary:
        reply["ok"] = false
        reply["error"] = "unsupported protocol version or data"
        reply["error_code"] = "unsupported_version" if request.get("version") != 1 else "invalid_request"
    elif request.get("command", "") != "hello" and request.get("command", "") != "shutdown" and not negotiated:
        reply["ok"] = false
        reply["error"] = "handshake required"
        reply["error_code"] = "invalid_state"
    else:
        match request.get("command", ""):
            "hello":
                if negotiated or not data.get("versions", []) is Array or not data.get("versions", []).has(1.0):
                    reply["ok"] = false
                    reply["error"] = "unsupported version or handshake already completed"
                    reply["error_code"] = "invalid_state" if negotiated else "unsupported_version"
                else:
                    negotiated = true
                    reply["capabilities"] = ["set_load", "reset", "metrics", "shutdown", "list_scenarios", "clock_sync", "cancel"]
                    reply["scenarios"] = LIMITS.keys()
                    reply["metric_metadata"] = METADATA
                    reply["metric_definitions"] = _metric_definitions()
                    reply["provider_info"] = {"name": "GDPL Godot example", "platform": OS.get_name(), "engine": "Godot", "process_id": str(OS.get_process_id()), "engine_version": str(Engine.get_version_info().get("string", "unknown")), "adapter_version": "0.1.0", "display_backend": DisplayServer.get_name(), "video_adapter": RenderingServer.get_video_adapter_name(), "video_vendor": RenderingServer.get_video_adapter_vendor(), "graphics_api": RenderingServer.get_video_adapter_api_version()}
            "clock_sync":
                reply["adapter_receive_us"] = receive_us
                reply["adapter_send_us"] = Time.get_ticks_usec()
            "list_scenarios":
                reply["scenarios"] = [{"id": "external_process", "workloads": LIMITS}]
            "initialize":
                var params = data.get("parameters", {})
                if initialized or data.get("scenario") != "external_process" or not params is Dictionary or params.keys().any(func(key): return key != "workload"):
                    reply["ok"] = false
                    reply["error"] = "invalid state, scenario or parameters"
                    reply["error_code"] = "invalid_state" if initialized else "unknown_scenario"
                else:
                    var kind := str(params.get("workload", "nodes"))
                    var ceiling := int(data.get("maximum_load", -1))
                    if not LIMITS.has(kind) or ceiling < 0 or ceiling > int(LIMITS.get(kind, -1)):
                        reply["ok"] = false
                        reply["error"] = "unknown workload or maximum exceeds its ceiling"
                    else:
                        workload_kind = kind
                        maximum_load = ceiling
                        initialized = true
            "reset", "cancel":
                if not initialized:
                    reply["ok"] = false
                    reply["error"] = "scenario not initialized"
                    reply["error_code"] = "invalid_state"
                else:
                    _clear_workload()
            "set_load":
                var load := int(data.get("load", -1))
                if not initialized or load < 0 or load > maximum_load:
                    reply["ok"] = false
                    reply["error"] = "scenario not initialized or load outside ceiling"
                else:
                    _clear_workload()
                    _create_workload(load)
                    await get_tree().process_frame
                    var observed := _workload_count()
                    if observed != load:
                        reply["ok"] = false
                        reply["error"] = "observed workload does not match requested load"
                    else:
                        reply["load"] = load
                        reply["observed_load"] = observed
                        reply["ready"] = true
            "metrics":
                if not initialized:
                    reply["ok"] = false
                    reply["error"] = "scenario not initialized"
                else:
                    reply["metrics"] = _metrics()
            "shutdown":
                _clear_workload()
                state_lock.lock()
                stopping = true
                state_lock.unlock()
            _:
                reply["ok"] = false
                reply["error"] = "unsupported command"
    if id is float or id is int:
        if id > last_request_id and id == floor(id): last_request_id = int(id)
    if not reply["ok"] and not reply.has("error_code"):
        match request.get("command", ""):
            "initialize": reply["error_code"] = "initialization_failed"
            "set_load": reply["error_code"] = "invalid_workload"
            "metrics": reply["error_code"] = "invalid_state"
            _: reply["error_code"] = "unsupported_capability"
    _reply(reply)
    gate.post()
    if stopping:
        reader.wait_to_finish()
        get_tree().quit()

func _metric_definitions() -> Dictionary:
    var units := {"godot_clock_us": "microseconds", "godot_process_interval_ms": "milliseconds", "godot_fps": "frames/second", "godot_process_ms": "milliseconds", "godot_physics_ms": "milliseconds", "godot_node_count": "nodes", "godot_workload_count": "objects", "godot_processing_nodes": "nodes", "godot_physics_active": "bodies", "godot_physics_pairs": "pairs", "godot_animation_playing": "players", "godot_animation_position_sum_s": "seconds", "godot_render_draw_calls": "draw_calls", "godot_render_objects": "objects", "godot_render_primitives": "primitives"}
    var result := {}
    for id in METADATA:
        result[id] = {"name": id, "unit": units[id], "data_type": "number", "source": "godot_adapter", "sampling_method": "polled snapshot", "timestamp_domain": "unavailable", "availability": "conditional", "diagnostic": METADATA[id]}
    return result

func _reply(message: Dictionary) -> void:
    # --no-header keeps startup output off the protocol stream. Do not use --quiet.
    print(JSON.stringify(message))

func _clear_workload() -> void:
    if workload_root == null:
        return
    for child in workload_root.get_children():
        workload_root.remove_child(child)
        child.free()

func _create_workload(load: int) -> void:
    match workload_kind:
        "nodes":
            for i in range(load):
                workload_root.add_child(ActiveNode.new())
        "physics":
            var floor_body := StaticBody3D.new()
            var floor_collision := CollisionShape3D.new()
            var floor_shape := BoxShape3D.new()
            floor_shape.size = Vector3(40, 1, 40)
            floor_collision.shape = floor_shape
            floor_body.position = Vector3(0, -5, 0)
            floor_body.add_child(floor_collision)
            workload_root.add_child(floor_body)
            var sphere := SphereShape3D.new()
            sphere.radius = 0.4
            for i in range(load):
                var body := RigidBody3D.new()
                body.can_sleep = false
                var collider := CollisionShape3D.new()
                collider.shape = sphere
                body.add_child(collider)
                body.position = Vector3(float(i % 8) - 4.0, float(i / 64) * 1.1, float((i / 8) % 8) - 4.0)
                workload_root.add_child(body)
        "rendering":
            var mesh := BoxMesh.new()
            var material := StandardMaterial3D.new()
            material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
            mesh.material = material
            for i in range(load):
                var instance := MeshInstance3D.new()
                instance.mesh = mesh
                instance.position = Vector3(float(i % 20) - 10.0, float(i / 20) - 8.0, 0)
                workload_root.add_child(instance)
        "animation":
            for i in range(load):
                var object := Node3D.new()
                var player := AnimationPlayer.new()
                var library := AnimationLibrary.new()
                var animation := Animation.new()
                animation.length = 2.0
                animation.loop_mode = Animation.LOOP_LINEAR
                var track := animation.add_track(Animation.TYPE_VALUE)
                animation.track_set_path(track, NodePath(".:rotation"))
                animation.track_insert_key(track, 0.0, Vector3.ZERO)
                animation.track_insert_key(track, 2.0, Vector3(0, TAU, 0))
                library.add_animation("rotate", animation)
                player.add_animation_library("", library)
                object.add_child(player)
                workload_root.add_child(object)
                player.play("rotate")

func _workload_count() -> int:
    var count := 0
    for child in workload_root.get_children():
        match workload_kind:
            "nodes":
                if child.get_script() == ActiveNode: count += 1
            "physics":
                if child is RigidBody3D: count += 1
            "rendering":
                if child is MeshInstance3D: count += 1
            "animation":
                if child is Node3D and child.get_child_count() > 0 and child.get_child(0) is AnimationPlayer: count += 1
    return count

func _metrics() -> Dictionary:
    var result := {"godot_clock_us": Time.get_ticks_usec(), "godot_node_count": Performance.get_monitor(Performance.OBJECT_NODE_COUNT), "godot_workload_count": _workload_count()}
    if workload_kind == "nodes":
        var processing := 0
        for child in workload_root.get_children():
            if child.is_processing(): processing += 1
        result["godot_processing_nodes"] = processing
    if workload_kind == "physics":
        result["godot_physics_active"] = Performance.get_monitor(Performance.PHYSICS_3D_ACTIVE_OBJECTS)
        result["godot_physics_pairs"] = Performance.get_monitor(Performance.PHYSICS_3D_COLLISION_PAIRS)
    if workload_kind == "animation":
        var playing := 0
        var position_sum := 0.0
        for child in workload_root.get_children():
            if child.get_child_count() > 0 and child.get_child(0) is AnimationPlayer:
                var player: AnimationPlayer = child.get_child(0)
                if player.is_playing(): playing += 1
                position_sum += player.current_animation_position
        result["godot_animation_playing"] = playing
        result["godot_animation_position_sum_s"] = position_sum
    if process_interval_ms > 0.0:
        result["godot_process_interval_ms"] = process_interval_ms
    var fps := Performance.get_monitor(Performance.TIME_FPS)
    if fps > 0.0:
        result["godot_fps"] = fps
    var process_seconds := Performance.get_monitor(Performance.TIME_PROCESS)
    if process_seconds > 0.0:
        result["godot_process_ms"] = process_seconds * 1000.0
    var physics_seconds := Performance.get_monitor(Performance.TIME_PHYSICS_PROCESS)
    if physics_seconds > 0.0:
        result["godot_physics_ms"] = physics_seconds * 1000.0
    if DisplayServer.get_name() != "headless":
        result["godot_render_draw_calls"] = Performance.get_monitor(Performance.RENDER_TOTAL_DRAW_CALLS_IN_FRAME)
        result["godot_render_objects"] = Performance.get_monitor(Performance.RENDER_TOTAL_OBJECTS_IN_FRAME)
        result["godot_render_primitives"] = Performance.get_monitor(Performance.RENDER_TOTAL_PRIMITIVES_IN_FRAME)
    return result
