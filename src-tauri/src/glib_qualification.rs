use glib::variant::ToVariant;

// Run optimized: the original immutable C out-pointer triggers UB under optimization.
#[test]
fn gtk_variant_string_out_pointer_survives_optimized_bidirectional_iteration() {
    let v = ["zero", "one", "two", "three", "four", "five"].to_variant();
    for _ in 0..1000 {
        let mut iter = v.array_iter_str().unwrap();
        assert_eq!(iter.next(), Some("zero"));
        assert_eq!(iter.next_back(), Some("five"));
        assert_eq!(iter.nth(1), Some("two"));
        assert_eq!(iter.nth_back(0), Some("four"));
        assert_eq!(iter.last(), Some("three"));
        assert_eq!(v.array_iter_str().unwrap().collect::<Vec<_>>(), ["zero", "one", "two", "three", "four", "five"]);
    }
}
#[test]
fn gtk_variant_string_iterator_handles_empty_unicode_and_wrong_type() {
    let empty: Vec<String> = vec![];
    assert_eq!(empty.to_variant().array_iter_str().unwrap().next(), None);
    let values = ["", "rain 🌧", "\"metadata\"", "लाइन"];
    let variant = values.to_variant();
    assert_eq!(variant.array_iter_str().unwrap().collect::<Vec<_>>(), values);
    assert!(123u32.to_variant().array_iter_str().is_err());
}
