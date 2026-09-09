// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

// oz/utils/ReentrancyGuard.sol

// OpenZeppelin Contracts (last updated v5.0.0) (utils/ReentrancyGuard.sol)

/**
 * @dev Contract module that helps prevent reentrant calls to a function.
 */
abstract contract ReentrancyGuard {
    uint256 private constant NOT_ENTERED = 1;
    uint256 private constant ENTERED = 2;

    uint256 private _status;

    /**
     * @dev Unauthorized reentrant call.
     */
    error ReentrancyGuardReentrantCall();

    constructor() {
        _status = NOT_ENTERED;
    }

    modifier nonReentrant() {
        _nonReentrantBefore();
        _;
        _nonReentrantAfter();
    }

    function _nonReentrantBefore() private {
        if (_status == ENTERED) {
            revert ReentrancyGuardReentrantCall();
        }
        _status = ENTERED;
    }

    function _nonReentrantAfter() private {
        _status = NOT_ENTERED;
    }

    function _reentrancyGuardEntered() internal view returns (bool) {
        return _status == ENTERED;
    }
}

// src/ReentrancyGuardHarness.sol

contract ReentrancyGuardHarness is ReentrancyGuard {
    // Empty bodies on purpose: a body that could revert on its own (`counter
    // += 1` overflows at the top of the range) would make the rules about
    // the body rather than about the modifier.
    function guarded() external nonReentrant {}

    function unguarded() external {}

    function entered() external view returns (bool) {
        return _reentrancyGuardEntered();
    }
}
