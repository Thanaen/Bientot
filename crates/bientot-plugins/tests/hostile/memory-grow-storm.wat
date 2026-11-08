;; expect: fails
;; memory: 16MiB
;; Grows its memory a page at a time and keeps asking once refused, until its fuel ends.
(module
  (memory (export "memory") 3 32)
  (func (export "bt_abi") (result i32) i32.const 1)
  ;; One buffer at a time is enough: these modules never read their events.
  (func (export "bt_alloc") (param i32) (result i32) i32.const 131072)
  (func (export "bt_free") (param i32 i32))
  (func (export "bt_handle") (param i32 i32) (result i64)
    (loop $more
      (drop (memory.grow (i32.const 1)))
      (drop (memory.grow (i32.const 65536)))
      (br $more))
    (i64.const 0)))
