;; expect: fails
;; A request of 10 000 nested arrays, against the JSON reader's recursion.
(module
  (import "bt:ui@1" "set" (func $set (param i32 i32) (result i64)))
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (func (export "bt_handle") (param i32 i32) (result i64)
    (memory.fill (i32.const 0) (i32.const 91) (i32.const 10000))
    (memory.fill (i32.const 10000) (i32.const 93) (i32.const 10000))
    (drop (call $set (i32.const 0) (i32.const 20000)))
    (i64.const 0)))
