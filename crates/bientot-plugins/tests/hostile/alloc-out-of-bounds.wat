;; expect: fails
;; bt_alloc hands the host a buffer far outside the memory.
(module
  (memory (export "memory") 1)
  (func (export "bt_abi") (result i32) i32.const 1)
  (func (export "bt_alloc") (param i32) (result i32) i32.const 2147418112)
  (func (export "bt_free") (param i32 i32))
  (func (export "bt_handle") (param i32 i32) (result i64) (i64.const 0)))
