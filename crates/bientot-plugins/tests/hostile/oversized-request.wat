;; expect: fails
;; A request of 70 000 bytes, above the 64 KiB a request may have.
(module
  (import "bt:ui@1" "set" (func $set (param i32 i32) (result i64)))
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (func (export "bt_handle") (param i32 i32) (result i64)
    (memory.fill (i32.const 0) (i32.const 32) (i32.const 70000))
    (drop (call $set (i32.const 0) (i32.const 70000)))
    (i64.const 0)))
