;; expect: fails
;; A start function that never returns: it runs before bt_abi, under the background fuel cap.
(module
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (func $forever (loop $more (br $more)))
  (start $forever)
  (func (export "bt_handle") (param i32 i32) (result i64) (i64.const 0)))
