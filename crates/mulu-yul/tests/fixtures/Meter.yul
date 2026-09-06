
/// @use-src 0:"Meter.sol"
object "Meter_31" {
    code {
        /// @src 0:302:519  "contract Meter {..."
        mstore(64, memoryguard(128))
        if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }

        constructor_Meter_31()

        let _1 := allocate_unbounded()
        codecopy(_1, dataoffset("Meter_31_deployed"), datasize("Meter_31_deployed"))

        return(_1, datasize("Meter_31_deployed"))

        function allocate_unbounded() -> memPtr {
            memPtr := mload(64)
        }

        function revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() {
            revert(0, 0)
        }

        /// @src 0:302:519  "contract Meter {..."
        function constructor_Meter_31() {

            /// @src 0:302:519  "contract Meter {..."

        }
        /// @src 0:302:519  "contract Meter {..."

    }
    /// @use-src 0:"Meter.sol"
    object "Meter_31_deployed" {
        code {
            /// @src 0:302:519  "contract Meter {..."
            mstore(64, memoryguard(128))

            if iszero(lt(calldatasize(), 4))
            {
                let selector := shift_right_224_unsigned(calldataload(0))
                switch selector

                case 0x0d611a05
                {
                    // record(uint8)

                    external_fun_record_20()
                }

                case 0x6014cddf
                {
                    // force(uint256)

                    external_fun_force_30()
                }

                case 0xfffd52c6
                {
                    // reading()

                    external_fun_reading_3()
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

            function cleanup_t_uint8(value) -> cleaned {
                cleaned := and(value, 0xff)
            }

            function validator_revert_t_uint8(value) {
                if iszero(eq(value, cleanup_t_uint8(value))) { revert(0, 0) }
            }

            function abi_decode_t_uint8(offset, end) -> value {
                value := calldataload(offset)
                validator_revert_t_uint8(value)
            }

            function abi_decode_tuple_t_uint8(headStart, dataEnd) -> value0 {
                if slt(sub(dataEnd, headStart), 32) { revert_error_dbdddcbe895c83990c08b3492a0e83918d802a52331272ac6fdb6a7c4aea3b1b() }

                {

                    let offset := 0

                    value0 := abi_decode_t_uint8(add(headStart, offset), dataEnd)
                }

            }

            function abi_encode_tuple__to__fromStack(headStart ) -> tail {
                tail := add(headStart, 0)

            }

            function external_fun_record_20() {

                if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }
                let param_0 :=  abi_decode_tuple_t_uint8(4, calldatasize())
                fun_record_20(param_0)
                let memPos := allocate_unbounded()
                let memEnd := abi_encode_tuple__to__fromStack(memPos  )
                return(memPos, sub(memEnd, memPos))

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

            function external_fun_force_30() {

                if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }
                let param_0 :=  abi_decode_tuple_t_uint256(4, calldatasize())
                fun_force_30(param_0)
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
            /// @src 0:323:345  "uint256 public reading"
            function getter_fun_reading_3() -> ret {

                let slot := 0
                let offset := 0

                ret := read_from_storage_split_dynamic_t_uint256(slot, offset)

            }
            /// @src 0:302:519  "contract Meter {..."

            function abi_encode_t_uint256_to_t_uint256_fromStack(value, pos) {
                mstore(pos, cleanup_t_uint256(value))
            }

            function abi_encode_tuple_t_uint256__to_t_uint256__fromStack(headStart , value0) -> tail {
                tail := add(headStart, 32)

                abi_encode_t_uint256_to_t_uint256_fromStack(value0,  add(headStart, 0))

            }

            function external_fun_reading_3() {

                if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }
                abi_decode_tuple_(4, calldatasize())
                let ret_0 :=  getter_fun_reading_3()
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

            function convert_t_rational_100_by_1_to_t_uint8(value) -> converted {
                converted := cleanup_t_uint8(identity(cleanup_t_rational_100_by_1(value)))
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

            function convert_t_uint8_to_t_uint256(value) -> converted {
                converted := cleanup_t_uint256(identity(cleanup_t_uint8(value)))
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

            /// @ast-id 20
            /// @src 0:352:448  "function record(uint8 x) external {..."
            function fun_record_20(var_x_5) {

                /// @src 0:404:405  "x"
                let _1 := var_x_5
                let expr_9 := _1
                /// @src 0:409:412  "100"
                let expr_10 := 0x64
                /// @src 0:404:412  "x <= 100"
                let expr_11 := iszero(gt(cleanup_t_uint8(expr_9), convert_t_rational_100_by_1_to_t_uint8(expr_10)))
                /// @src 0:396:420  "require(x <= 100, \"cap\")"
                require_helper_t_stringliteral_ee26143d8b4ce430ef218b5f9ac9945bc86f73e38d7546691ff1981ddb33c0a1(expr_11)
                /// @src 0:440:441  "x"
                let _2 := var_x_5
                let expr_16 := _2
                /// @src 0:430:441  "reading = x"
                let _3 := convert_t_uint8_to_t_uint256(expr_16)
                update_storage_value_offset_0_t_uint256_to_t_uint256(0x00, _3)
                let expr_17 := _3

            }
            /// @src 0:302:519  "contract Meter {..."

            /// @ast-id 30
            /// @src 0:454:517  "function force(uint256 x) external {..."
            function fun_force_30(var_x_22) {

                /// @src 0:509:510  "x"
                let _4 := var_x_22
                let expr_26 := _4
                /// @src 0:499:510  "reading = x"
                update_storage_value_offset_0_t_uint256_to_t_uint256(0x00, expr_26)
                let expr_27 := expr_26

            }
            /// @src 0:302:519  "contract Meter {..."

        }

        data ".metadata" hex"a2646970667358221220272d47e5372b852a6a5f51fe1546e51738e33c1813c51f8e0daa39e6b62fc10b64736f6c634300081c0033"
    }

}

