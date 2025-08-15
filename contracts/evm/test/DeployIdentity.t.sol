// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test, console2} from "forge-std-1.9.6/Test.sol";
import {IERC721Receiver} from "@openzeppelin-contracts-5.2.0/token/ERC721/IERC721Receiver.sol";
import {Identity} from "../src/Identity.sol";

contract DeployIdentityTest is Test, IERC721Receiver {
    bytes32 public constant TEST_SALT = keccak256("test");

    function testDeterministicDeployment() public {
        // Predict address before deployment
        address predicted = vm.computeCreate2Address(TEST_SALT, keccak256(type(Identity).creationCode), address(this));

        // Deploy using CREATE2
        Identity identity = new Identity{salt: TEST_SALT}();

        // Verify addresses match
        assertEq(address(identity), predicted, "Deployed address should match predicted");
    }

    function testSameSaltSameDeployer() public {
        // Two deployments with same salt and deployer should produce same address prediction
        address predicted1 = vm.computeCreate2Address(TEST_SALT, keccak256(type(Identity).creationCode), address(this));

        address predicted2 = vm.computeCreate2Address(TEST_SALT, keccak256(type(Identity).creationCode), address(this));

        assertEq(predicted1, predicted2, "Same salt and deployer should predict same address");
    }

    function testDifferentSaltsDifferentAddresses() public {
        bytes32 salt1 = keccak256("salt1");
        bytes32 salt2 = keccak256("salt2");

        address addr1 = vm.computeCreate2Address(salt1, keccak256(type(Identity).creationCode), address(this));

        address addr2 = vm.computeCreate2Address(salt2, keccak256(type(Identity).creationCode), address(this));

        assertTrue(addr1 != addr2, "Different salts should produce different addresses");
    }

    function testDifferentDeployersDifferentAddresses() public {
        address deployer1 = address(this);
        address deployer2 = makeAddr("deployer2");

        address addr1 = vm.computeCreate2Address(TEST_SALT, keccak256(type(Identity).creationCode), deployer1);

        address addr2 = vm.computeCreate2Address(TEST_SALT, keccak256(type(Identity).creationCode), deployer2);

        assertTrue(addr1 != addr2, "Different deployers should produce different addresses");
    }

    function testDeployedContractFunctionality() public {
        Identity identity = new Identity{salt: TEST_SALT}();

        // Test basic ERC721 functionality
        assertEq(identity.name(), "nXCC Identity");
        assertEq(identity.symbol(), "nxccid");

        // Test minting
        string memory policyURL = "https://example.com/policy.json";
        uint256 tokenId = identity.mint(policyURL);

        assertEq(tokenId, 1, "First token should have ID 1");
        assertEq(identity.ownerOf(tokenId), address(this), "Should own the minted token");
        assertEq(identity.tokenURI(tokenId), policyURL, "Token URI should match");
    }

    function onERC721Received(address, address, uint256, bytes calldata) external pure override returns (bytes4) {
        return IERC721Receiver.onERC721Received.selector;
    }
}
