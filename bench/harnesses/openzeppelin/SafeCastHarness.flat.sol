// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

// oz/utils/math/SafeCast.sol

// OpenZeppelin Contracts (last updated v5.0.0) (utils/math/SafeCast.sol)

/**
 * @dev Wrappers over Solidity's uintXX/intXX/bool casting operators with added overflow
 * checks.
 */
// Written as a contract rather than a library: solc emits a library as a
// separate deployed object and puts a `linkersymbol` in the caller, and the
// bodies under test are the same either way.
abstract contract SafeCastBase {
    error SafeCastOverflowedUintDowncast(uint8 bits, uint256 value);
    error SafeCastOverflowedIntToUint(int256 value);
    error SafeCastOverflowedIntDowncast(uint8 bits, int256 value);
    error SafeCastOverflowedUintToInt(uint256 value);

    function _toUint8(uint256 value) internal pure returns (uint8) {
        if (value > type(uint8).max) {
            revert SafeCastOverflowedUintDowncast(8, value);
        }
        return uint8(value);
    }

    function _toUint128(uint256 value) internal pure returns (uint128) {
        if (value > type(uint128).max) {
            revert SafeCastOverflowedUintDowncast(128, value);
        }
        return uint128(value);
    }

    function _toUint256(int256 value) internal pure returns (uint256) {
        if (value < 0) {
            revert SafeCastOverflowedIntToUint(value);
        }
        return uint256(value);
    }

    function _toInt8(int256 value) internal pure returns (int8 downcasted) {
        downcasted = int8(value);
        if (downcasted != value) {
            revert SafeCastOverflowedIntDowncast(8, value);
        }
    }

    function _toInt256(uint256 value) internal pure returns (int256) {
        if (value > uint256(type(int256).max)) {
            revert SafeCastOverflowedUintToInt(value);
        }
        return int256(value);
    }
}

// src/SafeCastHarness.sol

contract SafeCastHarness is SafeCastBase {
    function toUint8(uint256 value) external pure returns (uint8) {
        return _toUint8(value);
    }
    function toUint128(uint256 value) external pure returns (uint128) {
        return _toUint128(value);
    }
    function toUint256(int256 value) external pure returns (uint256) {
        return _toUint256(value);
    }
    function toInt8(int256 value) external pure returns (int8) {
        return _toInt8(value);
    }
    function toInt256(uint256 value) external pure returns (int256) {
        return _toInt256(value);
    }
}
