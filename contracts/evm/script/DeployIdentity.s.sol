// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script, console2} from "forge-std-1.9.6/Script.sol";
import {Identity} from "../src/Identity.sol";

/**
 * @title DeployIdentity
 * @notice Deployment script for Identity contracts using CREATE2 for deterministic addressing.
 * @dev Uses Foundry's built-in CREATE2 support with vm.createX opcodes.
 *      Run with: forge script script/DeployIdentity.s.sol --rpc-url <RPC_URL> --broadcast --verify
 */
contract DeployIdentity is Script {
    // Arachnid's Deterministic Deployment Proxy address
    // Available on most major EVM chains for universal CREATE2 deployment
    address public constant DDP_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;

    // Default salt for consistent deployment addresses across chains
    // Salt: 0x0 (zero bytes32)
    // Results in address: 0x843f604F71dDaaaE82a82551d6b19571E6C6E23A (via DDP)
    // Built with: Solc 0.8.28, release profile (4,294,967,295 optimizer runs, via-ir)
    // You can override this by setting IDENTITY_SALT environment variable
    bytes32 public constant DEFAULT_SALT = bytes32(0);

    function run() external {
        uint256 deployerPrivateKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerPrivateKey);

        console2.log("Deployer address:", deployer);
        console2.log("Deployer balance:", deployer.balance);

        // Get salt from environment or use default
        bytes32 salt = vm.envOr("IDENTITY_SALT", DEFAULT_SALT);

        console2.log("Salt used:", vm.toString(salt));

        vm.startBroadcast(deployerPrivateKey);

        // Deploy using CREATE2 with Foundry's vm.create2
        Identity identity = new Identity{salt: salt}();

        vm.stopBroadcast();

        console2.log("\n=== DEPLOYMENT SUMMARY ===");
        console2.log("Identity deployed to:", address(identity));
        console2.log("Deployment successful and deterministic!");

        // Test basic functionality
        console2.log("Identity name:", identity.name());
        console2.log("Identity symbol:", identity.symbol());

        // Show deterministic address using DDP
        address predictedDDPAddress = computeIdentityAddressViaDDP(salt);
        console2.log("\n=== DETERMINISTIC ADDRESS (DDP) ===");
        console2.log("Identity address via DDP:", predictedDDPAddress);
        console2.log("DDP deployer address:", DDP_DEPLOYER);
        console2.log("Salt:", vm.toString(salt));
        console2.log("");
        console2.log("This address will be the same on any chain with DDP deployed");
        console2.log("No specific deployer address required!");

        // Show how to predict this address on other chains
        console2.log("\n=== ADDRESS PREDICTION ===");
        console2.log("Current deployment (depends on deployer):");
        console2.log("1. Use the same deployer address:", deployer);
        console2.log("2. Use the same salt:", vm.toString(salt));
        console2.log("3. The contract will deploy to:", address(identity));
    }

    /**
     * @notice Helper function to compute Identity address for any deployer
     * @param deployer The address that will deploy the contract
     * @param salt The salt used for deployment
     * @return The predicted Identity address
     */
    function computeIdentityAddress(address deployer, bytes32 salt) external pure returns (address) {
        return vm.computeCreate2Address(salt, keccak256(type(Identity).creationCode), deployer);
    }

    /**
     * @notice Compute Identity address using Arachnid's Deterministic Deployment Proxy
     * @param salt The salt used for deployment
     * @return The predicted Identity address via DDP
     * @dev This address will be the same on any chain where DDP is deployed
     *      No specific deployer address required!
     */
    function computeIdentityAddressViaDDP(bytes32 salt) public pure returns (address) {
        return vm.computeCreate2Address(salt, keccak256(type(Identity).creationCode), DDP_DEPLOYER);
    }
}
