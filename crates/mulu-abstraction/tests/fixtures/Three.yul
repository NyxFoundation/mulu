
/// @use-src 0:"Three.sol"
object "Three_35" {
    code {
        /// @src 0:24:220  "contract Three {..."
        mstore(64, memoryguard(128))
        if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }

        constructor_Three_35()

        let _1 := allocate_unbounded()
        codecopy(_1, dataoffset("Three_35_deployed"), datasize("Three_35_deployed"))

        return(_1, datasize("Three_35_deployed"))

        function allocate_unbounded() -> memPtr {
            memPtr := mload(64)
        }

        function revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() {
            revert(0, 0)
        }

        /// @src 0:24:220  "contract Three {..."
        function constructor_Three_35() {

            /// @src 0:24:220  "contract Three {..."

        }
        /// @src 0:24:220  "contract Three {..."

    }
    /// @use-src 0:"Three.sol"
    object "Three_35_deployed" {
        code {
            /// @src 0:24:220  "contract Three {..."
            mstore(64, memoryguard(128))

            if iszero(lt(calldatasize(), 4))
            {
                let selector := shift_right_224_unsigned(calldataload(0))
                switch selector

                case 0xa4d66daf
                {
                    // limit()

                    external_fun_limit_3()
                }

                case 0xb3de648b
                {
                    // f(uint256)

                    external_fun_f_34()
                }

                default {}
            }

            revert_error_42b3090547df1d2001c96683413b8cf91c1b902ef5e3cb8d9f6f304cf7446f74()

            function shift_right_224_unsigned(value) -> newValue {
                newValue :=

                shr(224, value)

            }

            function allocate_unbounded() -> memPtr {
                memPtr := mload(64)
            }

            function revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() {
                revert(0, 0)
            }

            function revert_error_dbdddcbe895c83990c08b3492a0e83918d802a52331272ac6fdb6a7c4aea3b1b() {
                revert(0, 0)
            }

            function abi_decode_tuple_(headStart, dataEnd)   {
                if slt(sub(dataEnd, headStart), 0) { revert_error_dbdddcbe895c83990c08b3492a0e83918d802a52331272ac6fdb6a7c4aea3b1b() }

            }

            function shift_right_unsigned_dynamic(bits, value) -> newValue {
                newValue :=

                shr(bits, value)

            }

            function cleanup_from_storage_t_uint256(value) -> cleaned {
                cleaned := value
            }

            function extract_from_storage_value_dynamict_uint256(slot_value, offset) -> value {
                value := cleanup_from_storage_t_uint256(shift_right_unsigned_dynamic(mul(offset, 8), slot_value))
            }

            function read_from_storage_split_dynamic_t_uint256(slot, offset) -> value {
                value := extract_from_storage_value_dynamict_uint256(sload(slot), offset)

            }

            /// @ast-id 3
            /// @src 0:45:65  "uint256 public limit"
            function getter_fun_limit_3() -> ret {

                let slot := 0
                let offset := 0

                ret := read_from_storage_split_dynamic_t_uint256(slot, offset)

            }
            /// @src 0:24:220  "contract Three {..."

            function cleanup_t_uint256(value) -> cleaned {
                cleaned := value
            }

            function abi_encode_t_uint256_to_t_uint256_fromStack(value, pos) {
                mstore(pos, cleanup_t_uint256(value))
            }

            function abi_encode_tuple_t_uint256__to_t_uint256__fromStack(headStart , value0) -> tail {
                tail := add(headStart, 32)

                abi_encode_t_uint256_to_t_uint256_fromStack(value0,  add(headStart, 0))

            }

            function external_fun_limit_3() {

                if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }
                abi_decode_tuple_(4, calldatasize())
                let ret_0 :=  getter_fun_limit_3()
                let memPos := allocate_unbounded()
                let memEnd := abi_encode_tuple_t_uint256__to_t_uint256__fromStack(memPos , ret_0)
                return(memPos, sub(memEnd, memPos))

            }

            function revert_error_c1322bf8034eace5e0b5c7295db60986aa89aae5e0ea0873e4689e076861a5db() {
                revert(0, 0)
            }

            function validator_revert_t_uint256(value) {
                if iszero(eq(value, cleanup_t_uint256(value))) { revert(0, 0) }
            }

            function abi_decode_t_uint256(offset, end) -> value {
                value := calldataload(offset)
                validator_revert_t_uint256(value)
            }

            function abi_decode_tuple_t_uint256(headStart, dataEnd) -> value0 {
                if slt(sub(dataEnd, headStart), 32) { revert_error_dbdddcbe895c83990c08b3492a0e83918d802a52331272ac6fdb6a7c4aea3b1b() }

                {

                    let offset := 0

                    value0 := abi_decode_t_uint256(add(headStart, offset), dataEnd)
                }

            }

            function abi_encode_tuple__to__fromStack(headStart ) -> tail {
                tail := add(headStart, 0)

            }

            function external_fun_f_34() {

                if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }
                let param_0 :=  abi_decode_tuple_t_uint256(4, calldatasize())
                fun_f_34(param_0)
                let memPos := allocate_unbounded()
                let memEnd := abi_encode_tuple__to__fromStack(memPos  )
                return(memPos, sub(memEnd, memPos))

            }

            function revert_error_42b3090547df1d2001c96683413b8cf91c1b902ef5e3cb8d9f6f304cf7446f74() {
                revert(0, 0)
            }

            function cleanup_t_rational_10_by_1(value) -> cleaned {
                cleaned := value
            }

            function identity(value) -> ret {
                ret := value
            }

            function convert_t_rational_10_by_1_to_t_uint256(value) -> converted {
                converted := cleanup_t_uint256(identity(cleanup_t_rational_10_by_1(value)))
            }

            function array_storeLengthForEncoding_t_string_memory_ptr_fromStack(pos, length) -> updated_pos {
                mstore(pos, length)
                updated_pos := add(pos, 0x20)
            }

            function store_literal_in_memory_03783fac2efed8fbc9ad443e592ee30e61d65f471140c10ca155e937b435b760(memPtr) {

                mstore(add(memPtr, 0), "A")

            }

            function abi_encode_t_stringliteral_03783fac2efed8fbc9ad443e592ee30e61d65f471140c10ca155e937b435b760_to_t_string_memory_ptr_fromStack(pos) -> end {
                pos := array_storeLengthForEncoding_t_string_memory_ptr_fromStack(pos, 1)
                store_literal_in_memory_03783fac2efed8fbc9ad443e592ee30e61d65f471140c10ca155e937b435b760(pos)
                end := add(pos, 32)
            }

            function abi_encode_tuple_t_stringliteral_03783fac2efed8fbc9ad443e592ee30e61d65f471140c10ca155e937b435b760__to_t_string_memory_ptr__fromStack(headStart ) -> tail {
                tail := add(headStart, 32)

                mstore(add(headStart, 0), sub(tail, headStart))
                tail := abi_encode_t_stringliteral_03783fac2efed8fbc9ad443e592ee30e61d65f471140c10ca155e937b435b760_to_t_string_memory_ptr_fromStack( tail)

            }

            function require_helper_t_stringliteral_03783fac2efed8fbc9ad443e592ee30e61d65f471140c10ca155e937b435b760(condition ) {
                if iszero(condition)
                {

                    let memPtr := allocate_unbounded()

                    mstore(memPtr, 0x08c379a000000000000000000000000000000000000000000000000000000000)
                    let end := abi_encode_tuple_t_stringliteral_03783fac2efed8fbc9ad443e592ee30e61d65f471140c10ca155e937b435b760__to_t_string_memory_ptr__fromStack(add(memPtr, 4) )
                    revert(memPtr, sub(end, memPtr))
                }
            }

            function cleanup_t_rational_20_by_1(value) -> cleaned {
                cleaned := value
            }

            function convert_t_rational_20_by_1_to_t_uint256(value) -> converted {
                converted := cleanup_t_uint256(identity(cleanup_t_rational_20_by_1(value)))
            }

            function store_literal_in_memory_1f675bff07515f5df96737194ea945c36c41e7b4fcef307b7cd4d0e602a69111(memPtr) {

                mstore(add(memPtr, 0), "B")

            }

            function abi_encode_t_stringliteral_1f675bff07515f5df96737194ea945c36c41e7b4fcef307b7cd4d0e602a69111_to_t_string_memory_ptr_fromStack(pos) -> end {
                pos := array_storeLengthForEncoding_t_string_memory_ptr_fromStack(pos, 1)
                store_literal_in_memory_1f675bff07515f5df96737194ea945c36c41e7b4fcef307b7cd4d0e602a69111(pos)
                end := add(pos, 32)
            }

            function abi_encode_tuple_t_stringliteral_1f675bff07515f5df96737194ea945c36c41e7b4fcef307b7cd4d0e602a69111__to_t_string_memory_ptr__fromStack(headStart ) -> tail {
                tail := add(headStart, 32)

                mstore(add(headStart, 0), sub(tail, headStart))
                tail := abi_encode_t_stringliteral_1f675bff07515f5df96737194ea945c36c41e7b4fcef307b7cd4d0e602a69111_to_t_string_memory_ptr_fromStack( tail)

            }

            function require_helper_t_stringliteral_1f675bff07515f5df96737194ea945c36c41e7b4fcef307b7cd4d0e602a69111(condition ) {
                if iszero(condition)
                {

                    let memPtr := allocate_unbounded()

                    mstore(memPtr, 0x08c379a000000000000000000000000000000000000000000000000000000000)
                    let end := abi_encode_tuple_t_stringliteral_1f675bff07515f5df96737194ea945c36c41e7b4fcef307b7cd4d0e602a69111__to_t_string_memory_ptr__fromStack(add(memPtr, 4) )
                    revert(memPtr, sub(end, memPtr))
                }
            }

            function cleanup_t_rational_30_by_1(value) -> cleaned {
                cleaned := value
            }

            function convert_t_rational_30_by_1_to_t_uint256(value) -> converted {
                converted := cleanup_t_uint256(identity(cleanup_t_rational_30_by_1(value)))
            }

            function store_literal_in_memory_017e667f4b8c174291d1543c466717566e206df1bfd6f30271055ddafdb18f72(memPtr) {

                mstore(add(memPtr, 0), "C")

            }

            function abi_encode_t_stringliteral_017e667f4b8c174291d1543c466717566e206df1bfd6f30271055ddafdb18f72_to_t_string_memory_ptr_fromStack(pos) -> end {
                pos := array_storeLengthForEncoding_t_string_memory_ptr_fromStack(pos, 1)
                store_literal_in_memory_017e667f4b8c174291d1543c466717566e206df1bfd6f30271055ddafdb18f72(pos)
                end := add(pos, 32)
            }

            function abi_encode_tuple_t_stringliteral_017e667f4b8c174291d1543c466717566e206df1bfd6f30271055ddafdb18f72__to_t_string_memory_ptr__fromStack(headStart ) -> tail {
                tail := add(headStart, 32)

                mstore(add(headStart, 0), sub(tail, headStart))
                tail := abi_encode_t_stringliteral_017e667f4b8c174291d1543c466717566e206df1bfd6f30271055ddafdb18f72_to_t_string_memory_ptr_fromStack( tail)

            }

            function require_helper_t_stringliteral_017e667f4b8c174291d1543c466717566e206df1bfd6f30271055ddafdb18f72(condition ) {
                if iszero(condition)
                {

                    let memPtr := allocate_unbounded()

                    mstore(memPtr, 0x08c379a000000000000000000000000000000000000000000000000000000000)
                    let end := abi_encode_tuple_t_stringliteral_017e667f4b8c174291d1543c466717566e206df1bfd6f30271055ddafdb18f72__to_t_string_memory_ptr__fromStack(add(memPtr, 4) )
                    revert(memPtr, sub(end, memPtr))
                }
            }

            function shift_left_0(value) -> newValue {
                newValue :=

                shl(0, value)

            }

            function update_byte_slice_32_shift_0(value, toInsert) -> result {
                let mask := 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
                toInsert := shift_left_0(toInsert)
                value := and(value, not(mask))
                result := or(value, and(toInsert, mask))
            }

            function convert_t_uint256_to_t_uint256(value) -> converted {
                converted := cleanup_t_uint256(identity(cleanup_t_uint256(value)))
            }

            function prepare_store_t_uint256(value) -> ret {
                ret := value
            }

            function update_storage_value_offset_0_t_uint256_to_t_uint256(slot, value_0) {
                let convertedValue_0 := convert_t_uint256_to_t_uint256(value_0)
                sstore(slot, update_byte_slice_32_shift_0(sload(slot), prepare_store_t_uint256(convertedValue_0)))
            }

            /// @ast-id 34
            /// @src 0:71:218  "function f(uint256 x) external {..."
            function fun_f_34(var_x_5) {

                /// @src 0:120:121  "x"
                let _1 := var_x_5
                let expr_9 := _1
                /// @src 0:124:126  "10"
                let expr_10 := 0x0a
                /// @src 0:120:126  "x < 10"
                let expr_11 := lt(cleanup_t_uint256(expr_9), convert_t_rational_10_by_1_to_t_uint256(expr_10))
                /// @src 0:112:132  "require(x < 10, \"A\")"
                require_helper_t_stringliteral_03783fac2efed8fbc9ad443e592ee30e61d65f471140c10ca155e937b435b760(expr_11)
                /// @src 0:150:151  "x"
                let _2 := var_x_5
                let expr_16 := _2
                /// @src 0:154:156  "20"
                let expr_17 := 0x14
                /// @src 0:150:156  "x > 20"
                let expr_18 := gt(cleanup_t_uint256(expr_16), convert_t_rational_20_by_1_to_t_uint256(expr_17))
                /// @src 0:142:162  "require(x > 20, \"B\")"
                require_helper_t_stringliteral_1f675bff07515f5df96737194ea945c36c41e7b4fcef307b7cd4d0e602a69111(expr_18)
                /// @src 0:180:181  "x"
                let _3 := var_x_5
                let expr_23 := _3
                /// @src 0:184:186  "30"
                let expr_24 := 0x1e
                /// @src 0:180:186  "x < 30"
                let expr_25 := lt(cleanup_t_uint256(expr_23), convert_t_rational_30_by_1_to_t_uint256(expr_24))
                /// @src 0:172:192  "require(x < 30, \"C\")"
                require_helper_t_stringliteral_017e667f4b8c174291d1543c466717566e206df1bfd6f30271055ddafdb18f72(expr_25)
                /// @src 0:210:211  "x"
                let _4 := var_x_5
                let expr_30 := _4
                /// @src 0:202:211  "limit = x"
                update_storage_value_offset_0_t_uint256_to_t_uint256(0x00, expr_30)
                let expr_31 := expr_30

            }
            /// @src 0:24:220  "contract Three {..."

        }

        data ".metadata" hex"a26469706673582212205b5270eca7f8aafa762ba27bf874dd07fbcb03eb56a22d1c11668a93b8c862d764736f6c634300081c0033"
    }

}

