// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "@openzeppelin/contracts/token/ERC721/ERC721.sol";
import "@openzeppelin/contracts/access/Ownable.sol";

contract DemoNFT is ERC721, Ownable {
    uint256 private _nextTokenId = 1;

    mapping(uint256 => string) private _tokenMetadata;

    event NFTMoved(uint256 indexed tokenId, address indexed from, address indexed to, string toChain);

    constructor(string memory name, string memory symbol) ERC721(name, symbol) Ownable(msg.sender) {}

    function mint(address to, string memory metadata) public onlyOwner returns (uint256) {
        uint256 tokenId = _nextTokenId++;
        _mint(to, tokenId);
        _tokenMetadata[tokenId] = metadata;
        return tokenId;
    }

    function moveToChain(uint256 tokenId, string memory targetChain) public {
        require(ownerOf(tokenId) == msg.sender, "Not token owner");

        // Burn the token on this chain
        _burn(tokenId);

        // Emit event for NXCC worker to handle cross-chain transfer
        emit NFTMoved(tokenId, msg.sender, msg.sender, targetChain);
    }

    function getMetadata(uint256 tokenId) public view returns (string memory) {
        return _tokenMetadata[tokenId];
    }
}
