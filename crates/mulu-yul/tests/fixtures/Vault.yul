
/// @use-src 0:"Base.sol", 1:"Vault.sol"
object "Vault_53" {
    code {
        /// @src 1:47:503  "contract Vault is Bounded {..."
        mstore(64, memoryguard(128))
        if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }

        constructor_Vault_53()

        let _1 := allocate_unbounded()
        codecopy(_1, dataoffset("Vault_53_deployed"), datasize("Vault_53_deployed"))

        return(_1, datasize("Vault_53_deployed"))

        function allocate_unbounded() -> memPtr {
            memPtr := mload(64)
        }

        function revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() {
            revert(0, 0)
        }

        /// @src 1:47:503  "contract Vault is Bounded {..."
        function constructor_Vault_53() {

            /// @src 1:47:503  "contract Vault is Bounded {..."
            constructor_Bounded_15()

        }
        /// @src 1:47:503  "contract Vault is Bounded {..."

        /// @src 0:267:380  "abstract contract Bounded {..."
        function constructor_Bounded_15() {

            /// @src 0:267:380  "abstract contract Bounded {..."

        }
        /// @src 1:47:503  "contract Vault is Bounded {..."

    }
    /// @use-src 0:"Base.sol", 1:"Vault.sol"
    object "Vault_53_deployed" {
        code {
            /// @src 1:47:503  "contract Vault is Bounded {..."
            mstore(64, memoryguard(128))

            if iszero(lt(calldatasize(), 4))
            {
                let selector := shift_right_224_unsigned(calldataload(0))
                switch selector

                case 0x27ea6f2b
                {
                    // setLimit(uint256)

                    external_fun_setLimit_42()
                }

                case 0x81a9dc5e
                {
                    // forceSet(uint256)

                    external_fun_forceSet_52()
                }

                case 0xa4d66daf
                {
                    // limit()

                    external_fun_limit_22()
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

            function external_fun_setLimit_42() {

                if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }
                let param_0 :=  abi_decode_tuple_t_uint256(4, calldatasize())
                fun_setLimit_42(param_0)
                let memPos := allocate_unbounded()
                let memEnd := abi_encode_tuple__to__fromStack(memPos  )
                return(memPos, sub(memEnd, memPos))

            }

            function external_fun_forceSet_52() {

                if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }
                let param_0 :=  abi_decode_tuple_t_uint256(4, calldatasize())
                fun_forceSet_52(param_0)
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

            /// @ast-id 22
            /// @src 1:79:99  "uint256 public limit"
            function getter_fun_limit_22() -> ret {

                let slot := 0
                let offset := 0

                ret := read_from_storage_split_dynamic_t_uint256(slot, offset)

            }
            /// @src 1:47:503  "contract Vault is Bounded {..."

            function abi_encode_t_uint256_to_t_uint256_fromStack(value, pos) {
                mstore(pos, cleanup_t_uint256(value))
            }

            function abi_encode_tuple_t_uint256__to_t_uint256__fromStack(headStart , value0) -> tail {
                tail := add(headStart, 32)

                abi_encode_t_uint256_to_t_uint256_fromStack(value0,  add(headStart, 0))

            }

            function external_fun_limit_22() {

                if callvalue() { revert_error_ca66f745a3ce8ff40e2ccaf1ad45db7774001b90d25810abd9040049be7bf4bb() }
                abi_decode_tuple_(4, calldatasize())
                let ret_0 :=  getter_fun_limit_22()
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

            /// @ast-id 14
            /// @src 0:299:378  "modifier capped(uint256 x) {..."
            function modifier_capped_28(var_x_24) {

                /// @src 1:365:366  "x"
                let _1 := var_x_24
                let expr_27 := _1
                let _2 := expr_27
                let var_x_3 := _2

                /// @src 0:344:345  "x"
                let _3 := var_x_3
                let expr_6 := _3
                /// @src 0:349:352  "100"
                let expr_7 := 0x64
                /// @src 0:344:352  "x <= 100"
                let expr_8 := iszero(gt(cleanup_t_uint256(expr_6), convert_t_rational_100_by_1_to_t_uint256(expr_7)))
                /// @src 0:336:360  "require(x <= 100, \"cap\")"
                require_helper_t_stringliteral_ee26143d8b4ce430ef218b5f9ac9945bc86f73e38d7546691ff1981ddb33c0a1(expr_8)
                /// @src 0:370:371  "_"
                fun_setLimit_42_inner(var_x_24)

            }
            /// @src 1:47:503  "contract Vault is Bounded {..."

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

            /// @src 1:320:431  "function setLimit(uint256 x) external capped(x) {..."
            function fun_setLimit_42_inner(var_x_24) {

                /// @src 1:386:387  "x"
                let _4 := var_x_24
                let expr_31 := _4
                /// @src 1:391:395  "1000"
                let expr_32 := 0x03e8
                /// @src 1:386:395  "x <= 1000"
                let expr_33 := iszero(gt(cleanup_t_uint256(expr_31), convert_t_rational_1000_by_1_to_t_uint256(expr_32)))
                /// @src 1:378:405  "require(x <= 1000, \"bound\")"
                require_helper_t_stringliteral_b97928ba30b23bf25819f4a9f4ccc472cc947663c99afa50c4bae76f046ad1ac(expr_33)
                /// @src 1:423:424  "x"
                let _5 := var_x_24
                let expr_38 := _5
                /// @src 1:415:424  "limit = x"
                update_storage_value_offset_0_t_uint256_to_t_uint256(0x00, expr_38)
                let expr_39 := expr_38

            }
            /// @src 1:47:503  "contract Vault is Bounded {..."

            /// @ast-id 42
            /// @src 1:320:431  "function setLimit(uint256 x) external capped(x) {..."
            function fun_setLimit_42(var_x_24) {

                modifier_capped_28(var_x_24)
            }
            /// @src 1:47:503  "contract Vault is Bounded {..."

            /// @ast-id 52
            /// @src 1:437:501  "function forceSet(uint256 x) external {..."
            function fun_forceSet_52(var_x_44) {

                /// @src 1:493:494  "x"
                let _6 := var_x_44
                let expr_48 := _6
                /// @src 1:485:494  "limit = x"
                update_storage_value_offset_0_t_uint256_to_t_uint256(0x00, expr_48)
                let expr_49 := expr_48

            }
            /// @src 1:47:503  "contract Vault is Bounded {..."

        }

        data ".metadata" hex"a2646970667358221220bc0142598e32bfc868f6339841b393a9d68b78ba0b91b89715cc2052428606f364736f6c634300081c0033"
    }

}

