;; expect: fails
;; Direct recursion with no end and wide frames.
(module
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (func $down (param $n i64) (result i64)
    (local i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64 i64)
    (i64.add (call $down (i64.add (local.get $n) (i64.const 1))) (i64.const 1)))
  (func (export "bt_handle") (param i32 i32) (result i64)
    (drop (call $down (i64.const 0)))
    (i64.const 0)))
