// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.19;

contract TestEvents {
    event ValueChanged(uint256 indexed oldValue, uint256 indexed newValue, bytes data);
    event OtherEvent(uint256 indexed someValue);

    uint256 public value;
    bytes public eventDataPayload;

    function triggerEvent(uint256 _newValue, bytes calldata _data) public {
        emit ValueChanged(value, _newValue, _data);
    }

    function triggerOtherEvent(uint256 _someValue) public {
        emit OtherEvent(_someValue);
    }

    function updateState(uint256 _newValueFromEvent, bytes calldata _dataFromEvent) public {
        // Basic check to ensure it's not an accidental zero-value call,
        require(
            _newValueFromEvent != 0 || keccak256(_dataFromEvent) != keccak256(bytes("")),
            "Update with non-zero values or non-empty data"
        );
        value = _newValueFromEvent;
        eventDataPayload = _dataFromEvent;
    }

    function getValue() public view returns (uint256) {
        return value;
    }

    function getEventDataPayload() public view returns (bytes memory) {
        return eventDataPayload;
    }
}
