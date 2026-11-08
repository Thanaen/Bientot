;; expect: fails
;; An endless loop of tail calls through the table: no stack grows, only fuel ends it.
(module
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (type $void (func))
  (table 1 funcref)
  (elem (i32.const 0) $again)
  (func $again (return_call_indirect (type $void) (i32.const 0)))
  (func (export "bt_handle") (param i32 i32) (result i64)
    (call $again)
    (i64.const 0)))
