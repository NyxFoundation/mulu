
/// @use-src 0:"Limits.sol"
object "Limits_38" {
    code {
        /// @src 0:257:532  "contract Limits {..."
        mstore(64, memoryguard(128))
        if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }

        constructor_Limits_38()

        let _1 := allocate_unbounded()
        codecopy(_1, dataoffset("Limits_38_deployed"), datasize("Limits_38_deployed"))

        return(_1, datasize("Limits_38_deployed"))

        function allocate_unbounded() -> memPtr {
            memPtr := mload(64)
        }

        function revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() {
            revert(0, 0)
        }

        /// @src 0:257:532  "contract Limits {..."
        function constructor_Limits_38() {

            /// @src 0:257:532  "contract Limits {..."

        }
        /// @src 0:257:532  "contract Limits {..."

    }
    /// @use-src 0:"Limits.sol"
    object "Limits_38_deployed" {
        code {
            /// @src 0:257:532  "contract Limits {..."
            mstore(64, memoryguard(128))

            if iszero(lt(calldatasize(), 4))
            {
                let selector := shift_right_224_unsigned(calldataload(0))
                switch selector

                case 0x27ea6f2b
                {
                    // setLimit(uint256)

                    external_fun_setLimit_27()
                }

                case 0x81a9dc5e
                {
                    // forceSet(uint256)

                    external_fun_forceSet_37()
                }

                case 0xa4d66daf
                {
                    // limit()

                    external_fun_limit_3()
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

            function revert_error_c1322bf8034eace5e0b5c7295db60986aa89aae5e0ea0873e4689e076861a5db() {
                revert(0, 0)
            }

            function cleanup_t_uint256(value) -> cleaned {
                cleaned := value
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

            function external_fun_setLimit_27() {

                if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }
                let param_0 :=  abi_decode_tuple_t_uint256(4, calldatasize())
                fun_setLimit_27(param_0)
                let memPos := allocate_unbounded()
                let memEnd := abi_encode_tuple__to__fromStack(memPos  )
                return(memPos, sub(memEnd, memPos))

            }

            function external_fun_forceSet_37() {

                if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }
                let param_0 :=  abi_decode_tuple_t_uint256(4, calldatasize())
                fun_forceSet_37(param_0)
                let memPos := allocate_unbounded()
                let memEnd := abi_encode_tuple__to__fromStack(memPos  )
                return(memPos, sub(memEnd, memPos))

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
            /// @src 0:279:299  "uint256 public limit"
            function getter_fun_limit_3() -> ret {

                let slot := 0
                let offset := 0

                ret := read_from_storage_split_dynamic_t_uint256(slot, offset)

            }
            /// @src 0:257:532  "contract Limits {..."

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

            function revert_error_42b3090547df1d2001c96683413b8cf91c1b902ef5e3cb8d9f6f304cf7446f74() {
                revert(0, 0)
            }

            function cleanup_t_rational_100_by_1(value) -> cleaned {
                cleaned := value
            }

            function identity(value) -> ret {
                ret := value
            }

            function convert_t_rational_100_by_1_to_t_uint256(value) -> converted {
                converted := cleanup_t_uint256(identity(cleanup_t_rational_100_by_1(value)))
            }

            function array_storeLengthForEncoding_t_string_memory_ptr_fromStack(pos, length) -> updated_pos {
                mstore(pos, length)
                updated_pos := add(pos, 0x20)
            }

            function store_literal_in_memory_ee26143d8b4ce430ef218b5f9ac9945bc86f73e38d7546691ff1981ddb33c0a1(memPtr) {

                mstore(add(memPtr, 0), "cap")

            }

            function abi_encode_t_stringliteral_ee26143d8b4ce430ef218b5f9ac9945bc86f73e38d7546691ff1981ddb33c0a1_to_t_string_memory_ptr_fromStack(pos) -> end {
                pos := array_storeLengthForEncoding_t_string_memory_ptr_fromStack(pos, 3)
                store_literal_in_memory_ee26143d8b4ce430ef218b5f9ac9945bc86f73e38d7546691ff1981ddb33c0a1(pos)
                end := add(pos, 32)
            }

            function abi_encode_tuple_t_stringliteral_ee26143d8b4ce430ef218b5f9ac9945bc86f73e38d7546691ff1981ddb33c0a1__to_t_string_memory_ptr__fromStack(headStart ) -> tail {
                tail := add(headStart, 32)

                mstore(add(headStart, 0), sub(tail, headStart))
                tail := abi_encode_t_stringliteral_ee26143d8b4ce430ef218b5f9ac9945bc86f73e38d7546691ff1981ddb33c0a1_to_t_string_memory_ptr_fromStack( tail)

            }

            function require_helper_t_stringliteral_ee26143d8b4ce430ef218b5f9ac9945bc86f73e38d7546691ff1981ddb33c0a1(condition ) {
                if iszero(condition)
                {

                    let memPtr := allocate_unbounded()

                    mstore(memPtr, 0x08c379a000000000000000000000000000000000000000000000000000000000)
                    let end := abi_encode_tuple_t_stringliteral_ee26143d8b4ce430ef218b5f9ac9945bc86f73e38d7546691ff1981ddb33c0a1__to_t_string_memory_ptr__fromStack(add(memPtr, 4) )
                    revert(memPtr, sub(end, memPtr))
                }
            }

            function cleanup_t_rational_1000_by_1(value) -> cleaned {
                cleaned := value
            }

            function convert_t_rational_1000_by_1_to_t_uint256(value) -> converted {
                converted := cleanup_t_uint256(identity(cleanup_t_rational_1000_by_1(value)))
            }

            function store_literal_in_memory_b97928ba30b23bf25819f4a9f4ccc472cc947663c99afa50c4bae76f046ad1ac(memPtr) {

                mstore(add(memPtr, 0), "bound")

            }

            function abi_encode_t_stringliteral_b97928ba30b23bf25819f4a9f4ccc472cc947663c99afa50c4bae76f046ad1ac_to_t_string_memory_ptr_fromStack(pos) -> end {
                pos := array_storeLengthForEncoding_t_string_memory_ptr_fromStack(pos, 5)
                store_literal_in_memory_b97928ba30b23bf25819f4a9f4ccc472cc947663c99afa50c4bae76f046ad1ac(pos)
                end := add(pos, 32)
            }

            function abi_encode_tuple_t_stringliteral_b97928ba30b23bf25819f4a9f4ccc472cc947663c99afa50c4bae76f046ad1ac__to_t_string_memory_ptr__fromStack(headStart ) -> tail {
                tail := add(headStart, 32)

                mstore(add(headStart, 0), sub(tail, headStart))
                tail := abi_encode_t_stringliteral_b97928ba30b23bf25819f4a9f4ccc472cc947663c99afa50c4bae76f046ad1ac_to_t_string_memory_ptr_fromStack( tail)

            }

            function require_helper_t_stringliteral_b97928ba30b23bf25819f4a9f4ccc472cc947663c99afa50c4bae76f046ad1ac(condition ) {
                if iszero(condition)
                {

                    let memPtr := allocate_unbounded()

                    mstore(memPtr, 0x08c379a000000000000000000000000000000000000000000000000000000000)
                    let end := abi_encode_tuple_t_stringliteral_b97928ba30b23bf25819f4a9f4ccc472cc947663c99afa50c4bae76f046ad1ac__to_t_string_memory_ptr__fromStack(add(memPtr, 4) )
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

            /// @ast-id 27
            /// @src 0:306:460  "function setLimit(uint256 x) external {..."
            function fun_setLimit_27(var_x_5) {

                /// @src 0:362:363  "x"
                let _1 := var_x_5
                let expr_9 := _1
                /// @src 0:367:370  "100"
                let expr_10 := 0x64
                /// @src 0:362:370  "x <= 100"
                let expr_11 := iszero(gt(cleanup_t_uint256(expr_9), convert_t_rational_100_by_1_to_t_uint256(expr_10)))
                /// @src 0:354:378  "require(x <= 100, \"cap\")"
                require_helper_t_stringliteral_ee26143d8b4ce430ef218b5f9ac9945bc86f73e38d7546691ff1981ddb33c0a1(expr_11)
                /// @src 0:407:408  "x"
                let _2 := var_x_5
                let expr_16 := _2
                /// @src 0:412:416  "1000"
                let expr_17 := 0x03e8
                /// @src 0:407:416  "x <= 1000"
                let expr_18 := iszero(gt(cleanup_t_uint256(expr_16), convert_t_rational_1000_by_1_to_t_uint256(expr_17)))
                /// @src 0:399:426  "require(x <= 1000, \"bound\")"
                require_helper_t_stringliteral_b97928ba30b23bf25819f4a9f4ccc472cc947663c99afa50c4bae76f046ad1ac(expr_18)
                /// @src 0:452:453  "x"
                let _3 := var_x_5
                let expr_23 := _3
                /// @src 0:444:453  "limit = x"
                update_storage_value_offset_0_t_uint256_to_t_uint256(0x00, expr_23)
                let expr_24 := expr_23

            }
            /// @src 0:257:532  "contract Limits {..."

            /// @ast-id 37
            /// @src 0:466:530  "function forceSet(uint256 x) external {..."
            function fun_forceSet_37(var_x_29) {

                /// @src 0:522:523  "x"
                let _4 := var_x_29
                let expr_33 := _4
                /// @src 0:514:523  "limit = x"
                update_storage_value_offset_0_t_uint256_to_t_uint256(0x00, expr_33)
                let expr_34 := expr_33

            }
            /// @src 0:257:532  "contract Limits {..."

        }

        data ".metadata" hex"a26469706673582212200e9f7ae883f51b58fa0ba9ea112374e157dadaa16035cf1a1e025d33be07a3e864736f6c634300081c0033"
    }

}

