// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {ERC721} from "@openzeppelin-contracts-5.2.0/token/ERC721/ERC721.sol";
import {ERC721Burnable} from "@openzeppelin-contracts-5.2.0/token/ERC721/extensions/ERC721Burnable.sol";
import {ERC721URIStorage} from "@openzeppelin-contracts-5.2.0/token/ERC721/extensions/ERC721URIStorage.sol";

/**
 * @dev Interface for EIP-4907 (time-limited 'user' role).
 */
interface IERC4907 {
    event UpdateUser(uint256 indexed tokenId, address indexed user, uint64 expires);

    function setUser(uint256 tokenId, address user, uint64 expires) external;
    function userOf(uint256 tokenId) external view returns (address);
    function userExpires(uint256 tokenId) external view returns (uint256);
}

/**
 * @title (Machine) Identity
 * @notice An identity is represented as an ERC-721.
 *  - create an identity - mint
 *  - destroy an identity - burn
 *  - transfer an identity (e.g., to governance contract) - safeTransferFrom
 *  - reset off-chain policy - setPolicyURL
 *  - grant an address an on-chain badge - setUser (EIP-4907)
 *  - grant user rights using an on-chain "permitter" - approve
 */
contract Identity is ERC721, ERC721URIStorage, ERC721Burnable, IERC4907 {
    /**
     * @dev Records user information for each token.
     *      If `expires < block.timestamp`, `user` is considered unset.
     */
    struct UserInfo {
        address user;
        uint64 expires;
    }

    uint256 private _tokenIdCounter;
    mapping(uint256 => UserInfo) private _users;

    constructor() ERC721("nXCC Identity", "nxccid") {}

    /**
     * @notice Mints a new NFT to the caller with an initial metadata URI.
     * @param policyURL The initial metadata URI for the minted token.
     * @return tokenId The ID of the newly minted token.
     */
    function mint(string calldata policyURL) external returns (uint256 tokenId) {
        tokenId = ++_tokenIdCounter;
        _safeMint(msg.sender, tokenId);
        _setTokenURI(tokenId, policyURL);
    }

    /**
     * @notice Updates the token's metadata URI (policy URL).
     * @dev Caller must be owner or operator for the token.
     *      Emits a 4906 `MetadataUpdate` event via {ERC721URIStorage-_setTokenURI}.
     */
    function setPolicyURL(uint256 tokenId, string calldata newPolicyURL) external {
        _checkAuthorized(_requireOwned(tokenId), msg.sender, tokenId);
        _setTokenURI(tokenId, newPolicyURL);
    }

    /**
     * @notice Sets the `user` and `expires` for a token's EIP-4907 user role.
     * @dev Only the owner or an approved operator can call this.
     *      If `block.timestamp > expires`, user is effectively cleared.
     */
    function setUser(uint256 tokenId, address user, uint64 expires) external override {
        _checkAuthorized(_requireOwned(tokenId), msg.sender, tokenId);
        _users[tokenId] = UserInfo({user: user, expires: expires});
        emit UpdateUser(tokenId, user, expires);
    }

    /**
     * @return The address assigned as EIP-4907 `user`, or zero if expired.
     */
    function userOf(uint256 tokenId) public view override returns (address) {
        if (block.timestamp > _users[tokenId].expires) return address(0);
        return _users[tokenId].user;
    }

    /**
     * @return The timestamp at which the EIP-4907 `user` role expires.
     */
    function userExpires(uint256 tokenId) external view override returns (uint256) {
        return _users[tokenId].expires;
    }

    /**
     * @notice Returns metadata for multiple token IDs in one call.
     */
    function getTokenURIs(uint256[] calldata tokenIds) external view returns (string[] memory) {
        string[] memory uris = new string[](tokenIds.length);
        for (uint256 i; i < tokenIds.length; i++) {
            uris[i] = tokenURI(tokenIds[i]);
        }
        return uris;
    }

    /**
     * @dev Overridden to reset the EIP-4907 user info on transfers or burns.
     */
    function _update(address to, uint256 tokenId, address auth) internal virtual override(ERC721) returns (address) {
        address previousOwner = super._update(to, tokenId, auth);
        if (previousOwner != address(0) && to != previousOwner) {
            delete _users[tokenId];
            emit UpdateUser(tokenId, address(0), 0);
        }
        return previousOwner;
    }

    function tokenURI(uint256 tokenId) public view override(ERC721, ERC721URIStorage) returns (string memory) {
        return super.tokenURI(tokenId);
    }

    function supportsInterface(bytes4 interfaceId)
        public
        view
        virtual
        override(ERC721, ERC721URIStorage)
        returns (bool)
    {
        // EIP-4907: 0xad092b5c
        return interfaceId == type(IERC4907).interfaceId || super.supportsInterface(interfaceId);
    }
}
