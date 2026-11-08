;; expect: fails
;; Sends a card request cut in the middle.
(module
  (import "bt:ui@1" "set" (func $set (param i32 i32) (result i64)))
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (data (i32.const 0) "{\"slot\":\"flyout.card\",\"tree\":{\"type\":\"card\",\"li")
  (func (export "bt_handle") (param i32 i32) (result i64)
    (drop (call $set (i32.const 0) (i32.const 47)))
    (i64.const 0)))
