extends Node

var accumulator := 0.0

func _process(delta: float) -> void:
    accumulator = fmod(accumulator + delta, 10.0)
