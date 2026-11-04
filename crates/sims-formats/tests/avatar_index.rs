use sims_formats::avatar::index::IndexReader;

#[test]
fn unsafe_native_string_lengths_are_rejected_without_allocation() {
    // These are safety bounds, not equivalence claims about the original's
    // unchecked 1024-byte stack buffer or uninitialized partial length word.
    for size in [1024_u32, 65536, u32::MAX] {
        let mut data = 1_i32.to_le_bytes().to_vec();
        data.push(255);
        data.extend(size.to_le_bytes());
        let mut reader = IndexReader::new(&data);
        assert!(
            reader
                .next()
                .unwrap()
                .unwrap_err()
                .message
                .contains("capacity")
        );
        assert_eq!(reader.bytes_consumed(), 9);
        assert!(reader.next().is_none());
    }
    for available in 0..4 {
        let mut data = vec![1, 0, 0, 0, 255];
        data.extend(vec![1; available]);
        let mut reader = IndexReader::new(&data);
        assert_eq!(reader.next().unwrap().unwrap_err().offset, 5);
        assert_eq!(reader.bytes_consumed(), data.len());
        assert!(reader.next().is_none());
    }
}
