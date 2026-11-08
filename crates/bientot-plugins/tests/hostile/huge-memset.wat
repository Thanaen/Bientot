;; expect: fails
;; memory: 16MiB
;; Takes 16 MiB and fills all of it, again and again.
(module
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (func (export "bt_handle") (param i32 i32) (result i64)
    (drop (memory.grow (i32.sub (i32.const 256) (memory.size))))
    (loop $more
      (memory.fill (i32.const 0) (i32.const 171) (i32.mul (memory.size) (i32.const 65536)))
      (br $more))
    (i64.const 0)))
