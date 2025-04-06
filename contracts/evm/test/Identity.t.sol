// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std-1.9.6/Test.sol";
import {IERC721Errors} from "@openzeppelin-contracts-5.2.0/interfaces/draft-IERC6093.sol";

import {Identity} from "../src/Identity.sol";

contract IdentityTest is Test {
    Identity internal nft;
    address internal alice;
    address internal bob;
    address internal eve;

    // Events from ERC-721.
    event Transfer(address indexed from, address indexed to, uint256 indexed tokenId);

    function setUp() public {
        nft = new Identity();
        alice = makeAddr("alice");
        bob = makeAddr("bob");
        eve = makeAddr("eve");
    }

    function testMintSuccess() public {
        vm.startPrank(alice);
        uint256 tokenId = nft.mint("ipfs://initial-metadata");
        assertEq(tokenId, 1, "First minted token should have ID=1");
        assertEq(nft.ownerOf(tokenId), alice, "Ownership mismatch after mint");
        assertEq(nft.tokenURI(tokenId), "ipfs://initial-metadata");
        vm.stopPrank();
    }

    function testMintEmitsTransfer() public {
        vm.expectEmit(true, true, true, false);
        emit Transfer(address(0), alice, 1);

        vm.startPrank(alice);
        uint256 tokenId = nft.mint("example.com/1.json");
        vm.stopPrank();

        assertEq(tokenId, 1);
    }

    function testCannotSetPolicyURLIfNotApprovedOrOwner() public {
        vm.prank(alice);
        uint256 tokenId = nft.mint("orig.json");

        vm.prank(eve);
        vm.expectRevert(abi.encodeWithSelector(IERC721Errors.ERC721InsufficientApproval.selector, eve, 1));
        nft.setPolicyURL(tokenId, "evil-update.json");
    }

    function testSetPolicyURLAsOperator() public {
        vm.startPrank(alice);
        uint256 tokenId = nft.mint("ipfs://old");
        nft.approve(bob, tokenId);
        vm.stopPrank();

        vm.prank(bob);
        nft.setPolicyURL(tokenId, "ipfs://updated");
        assertEq(nft.tokenURI(tokenId), "ipfs://updated");
    }

    function testCannotBurnIfNotOwnerOrOperator() public {
        vm.prank(alice);
        uint256 tokenId = nft.mint("ipfs://burn-me");

        // Eve tries to burn.
        vm.prank(eve);
        vm.expectRevert(abi.encodeWithSelector(IERC721Errors.ERC721InsufficientApproval.selector, eve, 1));
        nft.burn(tokenId);

        // Confirm token still exists.
        vm.prank(alice);
        nft.burn(tokenId);
        vm.expectRevert(); // Token no longer exists.
        nft.ownerOf(tokenId);
    }

    function testSetUser() public {
        vm.startPrank(alice);
        uint256 tokenId = nft.mint("foo");
        nft.setUser(tokenId, bob, 9999999999);
        vm.stopPrank();

        // Check userOf.
        assertEq(nft.userOf(tokenId), bob);
    }

    function testSetUserExpiresInPast() public {
        vm.startPrank(alice);
        uint256 tokenId = nft.mint("foo");
        // Expire immediately.
        nft.setUser(tokenId, bob, uint64(block.timestamp - 1));
        vm.stopPrank();

        // Should be expired.
        assertEq(nft.userOf(tokenId), address(0));
    }

    function testClearUserOnTransfer() public {
        vm.startPrank(alice);
        uint256 tokenId = nft.mint("metadata");
        nft.setUser(tokenId, bob, 9999999999);
        vm.stopPrank();

        // Confirm user is set.
        assertEq(nft.userOf(tokenId), bob);

        // Alice transfers token to eve.
        vm.startPrank(alice);
        nft.transferFrom(alice, eve, tokenId);
        vm.stopPrank();

        // User info should be cleared.
        assertEq(nft.userOf(tokenId), address(0));
    }

    function testGetMultipleURIs() public {
        vm.startPrank(alice);
        nft.mint("ipfs://1");
        nft.mint("ipfs://2");
        nft.mint("ipfs://3");
        vm.stopPrank();

        uint256[] memory ids = new uint256[](3);
        ids[0] = 1;
        ids[1] = 2;
        ids[2] = 3;

        string[] memory uris = nft.getTokenURIs(ids);
        assertEq(uris[0], "ipfs://1");
        assertEq(uris[1], "ipfs://2");
        assertEq(uris[2], "ipfs://3");
    }

    function testUserExpiresCoverage() public {
        vm.startPrank(alice);
        uint256 tokenId = nft.mint("metadata");
        nft.setUser(tokenId, bob, 9999999999);
        vm.stopPrank();

        uint256 expires = nft.userExpires(tokenId);
        assertGt(expires, block.timestamp, "User expiry should be in the future");
    }

    function testSupportsInterface() public view {
        bool isERC721 = nft.supportsInterface(0x80ac58cd);
        bool isERC721Metadata = nft.supportsInterface(0x5b5e139f);
        bool isEIP4907 = nft.supportsInterface(0xad092b5c);
        bool isRandom = nft.supportsInterface(0xffffffff);

        assertTrue(isERC721, "Should support ERC721");
        assertTrue(isERC721Metadata, "Should support ERC721 Metadata");
        assertTrue(isEIP4907, "Should support EIP4907");
        assertFalse(isRandom, "Should not support random interface");
    }

    function testTransferToSelfDoesNotResetUserInfo() public {
        vm.startPrank(alice);
        uint256 tokenId = nft.mint("metadata");
        nft.setUser(tokenId, bob, 9999999999);
        // Transfer token to self.
        nft.transferFrom(alice, alice, tokenId);
        vm.stopPrank();

        // User info should persist.
        assertEq(nft.userOf(tokenId), bob, "User info should persist on self-transfer");
    }

    function testTokenURIFailsOnBurnedToken() public {
        vm.startPrank(alice);
        uint256 tokenId = nft.mint("ipfs://burn-me");
        nft.burn(tokenId);
        vm.stopPrank();

        vm.expectRevert(abi.encodeWithSelector(IERC721Errors.ERC721NonexistentToken.selector, 1));
        nft.tokenURI(tokenId);
    }

    function testTokenURIFailsOnNonexistentToken() public {
        vm.expectRevert(abi.encodeWithSelector(IERC721Errors.ERC721NonexistentToken.selector, 999));
        nft.tokenURI(999);
    }
}
