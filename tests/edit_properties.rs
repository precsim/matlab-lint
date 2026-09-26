use mstyle::edit::{Edit, EditError, apply_edits, normalize_edits};

fn next_u64(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

fn shuffle<T>(values: &mut [T], state: &mut u64) {
    for index in (1..values.len()).rev() {
        let other = (next_u64(state) as usize) % (index + 1);
        values.swap(index, other);
    }
}

#[test]
fn disjoint_edit_application_is_permutation_invariant() {
    for seed in 0_u64..512 {
        let mut state = seed ^ 0x9e37_79b9_7f4a_7c15;
        let len = 8 + (next_u64(&mut state) as usize % 96);
        let source: String = (0..len)
            .map(|index| char::from(b'a' + (index % 26) as u8))
            .collect();

        let mut edits = Vec::new();
        for start in (0..len).step_by(3) {
            if next_u64(&mut state) & 1 == 0 {
                continue;
            }
            let replacement_len = next_u64(&mut state) as usize % 4;
            let replacement: String = (0..replacement_len)
                .map(|_| char::from(b'A' + (next_u64(&mut state) % 26) as u8))
                .collect();
            edits.push(Edit::new(start, start + 1, replacement, "PROP"));
        }

        let expected = apply_edits(&source, edits.clone()).expect("disjoint edits must apply");
        for _ in 0..8 {
            let mut permuted = edits.clone();
            shuffle(&mut permuted, &mut state);
            let actual =
                apply_edits(&source, permuted).expect("permuted disjoint edits must apply");
            assert_eq!(actual, expected, "seed {seed}");
        }
    }
}

#[test]
fn normalization_is_sorted_and_idempotent_for_disjoint_edits() {
    for seed in 0_u64..256 {
        let mut state = seed.wrapping_add(17);
        let source_len = 64;
        let mut edits: Vec<_> = (0..source_len)
            .step_by(4)
            .filter(|_| next_u64(&mut state) & 1 == 1)
            .map(|start| Edit::new(start, start + 1, "x", "PROP"))
            .collect();

        shuffle(&mut edits, &mut state);
        let normalized =
            normalize_edits(source_len, edits).expect("generated edits are disjoint");
        assert!(
            normalized
                .windows(2)
                .all(|pair| pair[0].range.end <= pair[1].range.start)
        );

        let again =
            normalize_edits(source_len, normalized.clone()).expect("normalized edits remain valid");
        assert_eq!(again, normalized);
    }
}

#[test]
fn out_of_bounds_edits_are_always_rejected() {
    for source_len in 0..64 {
        for excess in 1..8 {
            let error = normalize_edits(
                source_len,
                vec![Edit::new(source_len, source_len + excess, "", "PROP")],
            )
            .expect_err("out-of-bounds edit must fail");
            assert!(matches!(error, EditError::InvalidRange { .. }));
        }
    }
}
