extends Node

const TRIALS := 5000
const DICE := 5

func _ready() -> void:
	print("=== Godot RNG fairness experiment ===")
	print("Godot version: ", Engine.get_version_info())
	print("Trials: %d, dice per roll: %d (1..6)" % [TRIALS, DICE])
	print("")

	_run_godot_rng()
	_run_better_rng()
	_run_godot_rng_reused()

	get_tree().quit()


func _run_godot_rng() -> void:
	var rolls: Array[PackedInt32Array] = []
	for t in TRIALS:
		var rng := RandomNumberGenerator.new()  # fresh instance per trial — the "reroll" pattern
		var r := PackedInt32Array()
		r.resize(DICE)
		for d in DICE:
			r[d] = rng.randi_range(1, 6)
		rolls.append(r)
	_report("RandomNumberGenerator (fresh per trial)", rolls)


func _run_better_rng() -> void:
	# BetterRng is registered by the gdextension; if it isn't, ClassDB.class_exists fails the test loudly.
	if not ClassDB.class_exists("BetterRng"):
		print("!! BetterRng class not found — extension didn't load")
		return
	var rolls: Array[PackedInt32Array] = []
	for t in TRIALS:
		var rng = ClassDB.instantiate("BetterRng")
		var r := PackedInt32Array()
		r.resize(DICE)
		for d in DICE:
			r[d] = rng.randi_range(1, 6)
		rolls.append(r)
	_report("BetterRng (fresh per trial)", rolls)


func _run_godot_rng_reused() -> void:
	# Control: a single RandomNumberGenerator reused. Should be fair.
	var rng := RandomNumberGenerator.new()
	rng.randomize()
	var rolls: Array[PackedInt32Array] = []
	for t in TRIALS:
		var r := PackedInt32Array()
		r.resize(DICE)
		for d in DICE:
			r[d] = rng.randi_range(1, 6)
		rolls.append(r)
	_report("RandomNumberGenerator (single instance, reused)", rolls)


func _report(label: String, rolls: Array) -> void:
	var consecutive_full := 0
	var consecutive_first := 0
	for t in range(1, rolls.size()):
		if _eq(rolls[t], rolls[t - 1]):
			consecutive_full += 1
		if rolls[t][0] == rolls[t - 1][0]:
			consecutive_first += 1
	var hist := {1: 0, 2: 0, 3: 0, 4: 0, 5: 0, 6: 0}
	for r in rolls:
		hist[r[0]] += 1
	var n: float = float(rolls.size() - 1)
	var expected_full: float = n / pow(6.0, DICE)
	var expected_first: float = n / 6.0
	print("== %s ==" % label)
	print("  consecutive full matches: %d  (expected ~%.2f)" % [consecutive_full, expected_full])
	print("  consecutive first-die matches: %d  (expected ~%.2f)" % [consecutive_first, expected_first])
	print("  first-die histogram: ", hist)
	print("")


func _eq(a: PackedInt32Array, b: PackedInt32Array) -> bool:
	if a.size() != b.size():
		return false
	for i in a.size():
		if a[i] != b[i]:
			return false
	return true
