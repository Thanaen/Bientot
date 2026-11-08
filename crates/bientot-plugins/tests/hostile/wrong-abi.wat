;; expect: fails
;; Speaks ABI 2.
(module
  (memory (export "memory") 1)
  (func (export "bt_abi") (result i32) i32.const 2)
  (func (export "bt_alloc") (param i32) (result i32) i32.const 0)
  (func (export "bt_free") (param i32 i32))
  (func (export "bt_handle") (param i32 i32) (result i64) (i64.const 0)))
