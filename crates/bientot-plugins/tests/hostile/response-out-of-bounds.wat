;; expect: fails
;; bt_handle answers with a response far outside the memory.
(module
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (func (export "bt_handle") (param i32 i32) (result i64)
    (i64.or (i64.shl (i64.const 2147418112) (i64.const 32)) (i64.const 4096))))
