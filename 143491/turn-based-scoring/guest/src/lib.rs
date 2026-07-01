#![cfg_attr(feature = "guest", no_std)]

// ============================================================================
// Test case: issue-143491 / new-turn-based-scoring
// Rust issue: https://github.com/rust-lang/rust/issues/143491
//
// WHAT THIS DOES (real-world zkVM workload):
//   Proves the result of a two-player turn-based match from its committed move
//   list (the "program / computation verification" archetype — a verifiable game
//   or tournament outcome). Players P0 (false) and P1 (true) alternate. For each
//   ply we set up whose turn comes next, evaluate the current move's point value,
//   then credit those points to the player who is on move now. The two reads of
//   the turn bit are naturally separated by the move-evaluation work.
//
// WHY IT TRIGGERS THE MISCOMPILATION (code pattern):
//   The turn bit lives in `turn_cell`, reached through the `&bool` cursor `turn`.
//   Read 1 (`next_turn = !*turn`) is computed first and is what gets stored into
//   the cell; the move scoring touches only `mv`/`points`; then read 2 (`mover =
//   *turn`) picks who gets the points, before `turn_cell = next_turn; turn =
//   &turn_cell` (`c = a; p = &c`). Non-adjacent reads, but the load-bearing order
//   holds: the cell-feeding read is first, and nothing between the reads writes
//   the cell or reassigns the cursor. On rustc 1.94 (fixed by PR #143509) MIR
//   CopyProp folds `turn_cell` into `next_turn`; the cursor aliases it, so
//   `mover` reads the already-advanced turn bit. Correct: `mover` is the player
//   on move before the turn flips; buggy: it is the player after the flip
//   (inverted), so every move from the second onward is credited to the opponent.
//
// SECURITY IMPACT:
//   A VALID proof attests a final score (and therefore a winner) that the move
//   list does not produce — points are credited to the wrong player — so a
//   verifier accepts a forged game/tournament outcome with the proof validating.
// ============================================================================

#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn score_match(moves: [u32; 32], seed: bool) -> u64 {
    let mut turn_cell;            // backing cell for the turn bit
    let mut turn = &seed;         // cursor: whose move it is right now

    let mut score0: u64 = 0;
    let mut score1: u64 = 0;

    let mut i = 0;
    while i < moves.len() {
        let next_turn = !*turn;   // read 1: who is on move next ply

        // ---- evaluate the current move (independent of whose turn it is) ----
        let mv = moves[i];
        let points = ((mv & 0xFF) ^ ((mv >> 8) & 0xFF)).count_ones() as u64
            + (mv % 7) as u64;
        // --------------------------------------------------------------------

        let mover = *turn;        // read 2: player who made THIS move
        turn_cell = next_turn;    // advance the cell    (c = a)
        turn = &turn_cell;        // re-point the cursor (p = &c)

        if mover {
            score1 += points;
        } else {
            score0 += points;
        }
        i += 1;
    }

    // Encode the outcome: top bit = winner (1 => P1), low bits = score margin.
    if score0 >= score1 {
        score0 - score1
    } else {
        (1u64 << 63) | (score1 - score0)
    }
}
