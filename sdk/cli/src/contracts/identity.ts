// Auto-generated file - do not edit manually
// Generated from contracts/evm/out/Identity.sol/Identity.json

export const IDENTITY_ABI = [
  {
    type: "constructor",
    inputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "approve",
    inputs: [
      {
        name: "to",
        type: "address",
        internalType: "address",
      },
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    outputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "balanceOf",
    inputs: [
      {
        name: "owner",
        type: "address",
        internalType: "address",
      },
    ],
    outputs: [
      {
        name: "",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "burn",
    inputs: [
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    outputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "getApproved",
    inputs: [
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    outputs: [
      {
        name: "",
        type: "address",
        internalType: "address",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "getTokenURIs",
    inputs: [
      {
        name: "tokenIds",
        type: "uint256[]",
        internalType: "uint256[]",
      },
    ],
    outputs: [
      {
        name: "",
        type: "string[]",
        internalType: "string[]",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "isApprovedForAll",
    inputs: [
      {
        name: "owner",
        type: "address",
        internalType: "address",
      },
      {
        name: "operator",
        type: "address",
        internalType: "address",
      },
    ],
    outputs: [
      {
        name: "",
        type: "bool",
        internalType: "bool",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "mint",
    inputs: [
      {
        name: "policyURL",
        type: "string",
        internalType: "string",
      },
    ],
    outputs: [
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "name",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "string",
        internalType: "string",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "ownerOf",
    inputs: [
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    outputs: [
      {
        name: "",
        type: "address",
        internalType: "address",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "safeTransferFrom",
    inputs: [
      {
        name: "from",
        type: "address",
        internalType: "address",
      },
      {
        name: "to",
        type: "address",
        internalType: "address",
      },
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    outputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "safeTransferFrom",
    inputs: [
      {
        name: "from",
        type: "address",
        internalType: "address",
      },
      {
        name: "to",
        type: "address",
        internalType: "address",
      },
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
      {
        name: "data",
        type: "bytes",
        internalType: "bytes",
      },
    ],
    outputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "setApprovalForAll",
    inputs: [
      {
        name: "operator",
        type: "address",
        internalType: "address",
      },
      {
        name: "approved",
        type: "bool",
        internalType: "bool",
      },
    ],
    outputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "setPolicyURL",
    inputs: [
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
      {
        name: "newPolicyURL",
        type: "string",
        internalType: "string",
      },
    ],
    outputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "setUser",
    inputs: [
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
      {
        name: "user",
        type: "address",
        internalType: "address",
      },
      {
        name: "expires",
        type: "uint64",
        internalType: "uint64",
      },
    ],
    outputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "supportsInterface",
    inputs: [
      {
        name: "interfaceId",
        type: "bytes4",
        internalType: "bytes4",
      },
    ],
    outputs: [
      {
        name: "",
        type: "bool",
        internalType: "bool",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "symbol",
    inputs: [],
    outputs: [
      {
        name: "",
        type: "string",
        internalType: "string",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "tokenURI",
    inputs: [
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    outputs: [
      {
        name: "",
        type: "string",
        internalType: "string",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "transferFrom",
    inputs: [
      {
        name: "from",
        type: "address",
        internalType: "address",
      },
      {
        name: "to",
        type: "address",
        internalType: "address",
      },
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    outputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "userExpires",
    inputs: [
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    outputs: [
      {
        name: "",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "userOf",
    inputs: [
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
    ],
    outputs: [
      {
        name: "",
        type: "address",
        internalType: "address",
      },
    ],
    stateMutability: "view",
  },
  {
    type: "event",
    name: "Approval",
    inputs: [
      {
        name: "owner",
        type: "address",
        indexed: true,
        internalType: "address",
      },
      {
        name: "approved",
        type: "address",
        indexed: true,
        internalType: "address",
      },
      {
        name: "tokenId",
        type: "uint256",
        indexed: true,
        internalType: "uint256",
      },
    ],
    anonymous: false,
  },
  {
    type: "event",
    name: "ApprovalForAll",
    inputs: [
      {
        name: "owner",
        type: "address",
        indexed: true,
        internalType: "address",
      },
      {
        name: "operator",
        type: "address",
        indexed: true,
        internalType: "address",
      },
      {
        name: "approved",
        type: "bool",
        indexed: false,
        internalType: "bool",
      },
    ],
    anonymous: false,
  },
  {
    type: "event",
    name: "BatchMetadataUpdate",
    inputs: [
      {
        name: "_fromTokenId",
        type: "uint256",
        indexed: false,
        internalType: "uint256",
      },
      {
        name: "_toTokenId",
        type: "uint256",
        indexed: false,
        internalType: "uint256",
      },
    ],
    anonymous: false,
  },
  {
    type: "event",
    name: "MetadataUpdate",
    inputs: [
      {
        name: "_tokenId",
        type: "uint256",
        indexed: false,
        internalType: "uint256",
      },
    ],
    anonymous: false,
  },
  {
    type: "event",
    name: "Transfer",
    inputs: [
      {
        name: "from",
        type: "address",
        indexed: true,
        internalType: "address",
      },
      {
        name: "to",
        type: "address",
        indexed: true,
        internalType: "address",
      },
      {
        name: "tokenId",
        type: "uint256",
        indexed: true,
        internalType: "uint256",
      },
    ],
    anonymous: false,
  },
  {
    type: "event",
    name: "UpdateUser",
    inputs: [
      {
        name: "tokenId",
        type: "uint256",
        indexed: true,
        internalType: "uint256",
      },
      {
        name: "user",
        type: "address",
        indexed: true,
        internalType: "address",
      },
      {
        name: "expires",
        type: "uint64",
        indexed: false,
        internalType: "uint64",
      },
    ],
    anonymous: false,
  },
  {
    type: "error",
    name: "ERC721IncorrectOwner",
    inputs: [
      {
        name: "sender",
        type: "address",
        internalType: "address",
      },
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
      {
        name: "owner",
        type: "address",
        internalType: "address",
      },
    ],
  },
  {
    type: "error",
    name: "ERC721InsufficientApproval",
    inputs: [
      {
        name: "operator",
        type: "address",
        internalType: "address",
      },
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
    ],
  },
  {
    type: "error",
    name: "ERC721InvalidApprover",
    inputs: [
      {
        name: "approver",
        type: "address",
        internalType: "address",
      },
    ],
  },
  {
    type: "error",
    name: "ERC721InvalidOperator",
    inputs: [
      {
        name: "operator",
        type: "address",
        internalType: "address",
      },
    ],
  },
  {
    type: "error",
    name: "ERC721InvalidOwner",
    inputs: [
      {
        name: "owner",
        type: "address",
        internalType: "address",
      },
    ],
  },
  {
    type: "error",
    name: "ERC721InvalidReceiver",
    inputs: [
      {
        name: "receiver",
        type: "address",
        internalType: "address",
      },
    ],
  },
  {
    type: "error",
    name: "ERC721InvalidSender",
    inputs: [
      {
        name: "sender",
        type: "address",
        internalType: "address",
      },
    ],
  },
  {
    type: "error",
    name: "ERC721NonexistentToken",
    inputs: [
      {
        name: "tokenId",
        type: "uint256",
        internalType: "uint256",
      },
    ],
  },
] as const;

export const IDENTITY_BYTECODE =
  "0x60806040523461031457604080519081016001600160401b0381118282101761022a576040908152600d82526c6e584343204964656e7469747960981b602083015280519081016001600160401b0381118282101761022a5760405260068152651b9e18d8da5960d21b602082015281516001600160401b03811161022a575f54600181811c9116801561030a575b602082101461020c57601f81116102a8575b50602092601f821160011461024957928192935f9261023e575b50508160011b915f199060031b1c1916175f555b80516001600160401b03811161022a57600154600181811c91168015610220575b602082101461020c57601f81116101a9575b50602091601f8211600114610149579181925f9261013e575b50508160011b915f199060031b1c1916176001555b604051611ea590816103198239f35b015190505f8061011a565b601f1982169260015f52805f20915f5b85811061019157508360019510610179575b505050811b0160015561012f565b01515f1960f88460031b161c191690555f808061016b565b91926020600181928685015181550194019201610159565b60015f527fb10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6601f830160051c81019160208410610202575b601f0160051c01905b8181106101f75750610101565b5f81556001016101ea565b90915081906101e1565b634e487b7160e01b5f52602260045260245ffd5b90607f16906100ef565b634e487b7160e01b5f52604160045260245ffd5b015190505f806100ba565b601f198216935f8052805f20915f5b8681106102905750836001959610610278575b505050811b015f556100ce565b01515f1960f88460031b161c191690555f808061026b565b91926020600181928685015181550194019201610258565b5f80527f290decd9548b62a8d60345a988386fc84ba6bc95484008f6362f93160ef3e563601f830160051c81019160208410610300575b601f0160051c01905b8181106102f557506100a0565b5f81556001016102e8565b90915081906102df565b90607f169061008e565b5f80fdfe6080806040526004361015610012575f80fd5b5f3560e01c90816301ffc9a71461110f5750806306fdde031461104f578063081812fc14610fe7578063095ea7b314610e6b57806323b872dd14610e545780632fbfe73614610de257806342842e0e14610db357806342966c6814610d785780636352211e14610d3c57806370a0823114610ca75780638fc88c4814610c5257806395d89b4114610b4c578063a22cb46514610a4f578063b88d4fde146109c3578063bf72960714610835578063c2f1f14a146107db578063c87b56dd14610786578063d85d3d2714610322578063e030565e1461018d5763e985e9c5146100f8575f80fd5b346101895760407ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc3601126101895761012f6112a5565b73ffffffffffffffffffffffffffffffffffffffff61014c6112c8565b91165f52600560205273ffffffffffffffffffffffffffffffffffffffff60405f2091165f52602052602060ff60405f2054166040519015158152f35b5f80fd5b346101895760607ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc360112610189576004356101c76112c8565b9060443567ffffffffffffffff8116809103610189576101f1826101ea8161170b565b33906118df565b60405192604084019184831067ffffffffffffffff8411176102f55773ffffffffffffffffffffffffffffffffffffffff6020927f4e06b4e7000e659094299b3533b47b6aa8ad048e95e872d23d1f4ee55af89cfe946040521694858152828101828152855f526008845273ffffffffffffffffffffffffffffffffffffffff8060405f20935116167fffffffffffffffffffffffff0000000000000000000000000000000000000000835416178255517fffffffff0000000000000000ffffffffffffffffffffffffffffffffffffffff7bffffffffffffffff000000000000000000000000000000000000000083549260a01b169116179055604051908152a3005b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b346101895760207ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc3601126101895760043567ffffffffffffffff81116101895761037190369060040161135d565b906007547fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff81146107595760010190816007556020926040516103b4858261138b565b5f8152331561072d57835f526002855273ffffffffffffffffffffffffffffffffffffffff60405f20541680151590816106c0575b335f526003875260405f2060018154019055855f526002875260405f2073ffffffffffffffffffffffffffffffffffffffff33167fffffffffffffffffffffffff000000000000000000000000000000000000000082541617905586866040518133857fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef5f80a484806106b6575b610679575b5050505061064d578490333b6104ac575b50506104a49161049e913691611406565b826119f3565b604051908152f35b906104fe91604051809381927f150b7a020000000000000000000000000000000000000000000000000000000083523360048401525f6024840152886044840152608060648401526084830190611262565b03815f335af15f91816105f5575b5061057757843d15610570573d610522816113cc565b90610530604051928361138b565b81523d5f8383013e5b8051918261056d577f64a0ae92000000000000000000000000000000000000000000000000000000005f523360045260245ffd5b01fd5b6060610539565b7fffffffff000000000000000000000000000000000000000000000000000000007f150b7a02000000000000000000000000000000000000000000000000000000009116036105c9578361049e61048d565b7f64a0ae92000000000000000000000000000000000000000000000000000000005f523360045260245ffd5b9091508581813d8311610646575b61060d818361138b565b8101031261018957517fffffffff000000000000000000000000000000000000000000000000000000008116810361018957908661050c565b503d610603565b7f73c6ac6e000000000000000000000000000000000000000000000000000000005f525f60045260245ffd5b5f927f4e06b4e7000e659094299b3533b47b6aa8ad048e95e872d23d1f4ee55af89cfe9183855260088252846040812055848152a386868961047c565b5083331415610477565b6106f7865f52600460205260405f207fffffffffffffffffffffffff00000000000000000000000000000000000000008154169055565b805f526003875260405f207fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff81540190556103e9565b7f64a0ae92000000000000000000000000000000000000000000000000000000005f525f60045260245ffd5b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b346101895760207ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc360112610189576107d76107c3600435611d8c565b604051918291602083526020830190611262565b0390f35b346101895760207ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc3601126101895760206108176004356116c1565b73ffffffffffffffffffffffffffffffffffffffff60405191168152f35b346101895760207ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc3601126101895760043567ffffffffffffffff8111610189573660238201121561018957806004013567ffffffffffffffff8111610189573660248260051b84010111610189576108ad81611668565b916108bb604051938461138b565b8183527fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe06108e883611668565b015f5b8181106109b25750505f5b8281101561092f5760019061091360248260051b85010135611d8c565b61091d8287611680565b526109288186611680565b50016108f6565b836040518091602082016020835281518091526040830190602060408260051b8601019301915f905b82821061096757505050500390f35b919360206109a2827fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc060019597998495030186528851611262565b9601920192018594939192610958565b8060606020809388010152016108eb565b346101895760807ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc360112610189576109fa6112a5565b610a026112c8565b906044356064359267ffffffffffffffff8411610189573660238501121561018957610a3b610a4d943690602481600401359101611406565b92610a4783838361148d565b33611bb5565b005b346101895760407ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc36011261018957610a866112a5565b602435908115158092036101895773ffffffffffffffffffffffffffffffffffffffff16908115610b2057335f52600560205260405f20825f5260205260405f207fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff0081541660ff83161790556040519081527f17307eab39ab6107e8899845ad3d59bd9653f200f220920489ca2b5937696c3160203392a3005b507f5b08ba18000000000000000000000000000000000000000000000000000000005f5260045260245ffd5b34610189575f7ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc360112610189576040515f600154610b8a8161143c565b8084529060018116908115610c105750600114610bb2575b6107d7836107c38185038261138b565b91905060015f527fb10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6915f905b808210610bf6575090915081016020016107c3610ba2565b919260018160209254838588010152019101909291610bde565b7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff001660208086019190915291151560051b840190910191506107c39050610ba2565b346101895760207ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc360112610189576004355f526008602052602067ffffffffffffffff60405f205460a01c16604051908152f35b346101895760207ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc3601126101895773ffffffffffffffffffffffffffffffffffffffff610cf36112a5565b168015610d10575f526003602052602060405f2054604051908152f35b7f89c62b64000000000000000000000000000000000000000000000000000000005f525f60045260245ffd5b346101895760207ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc36011261018957602061081760043561170b565b346101895760207ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc36011261018957610a4d33600435611764565b3461018957610a4d610dc4366112eb565b9060405192610dd460208561138b565b5f8452610a4783838361148d565b346101895760407ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc3601126101895760243560043567ffffffffffffffff821161018957610e4e610e3a610a4d93369060040161135d565b610e47846101ea8161170b565b3691611406565b906119f3565b3461018957610a4d610e65366112eb565b9161148d565b346101895760407ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc36011261018957610ea26112a5565b602435610eae8161170b565b33151580610fc7575b80610f7a575b610f4e57819073ffffffffffffffffffffffffffffffffffffffff80851691167f8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b9255f80a45f52600460205273ffffffffffffffffffffffffffffffffffffffff60405f2091167fffffffffffffffffffffffff00000000000000000000000000000000000000008254161790555f80f35b7fa9fbf51f000000000000000000000000000000000000000000000000000000005f523360045260245ffd5b5073ffffffffffffffffffffffffffffffffffffffff81165f52600560205260405f2073ffffffffffffffffffffffffffffffffffffffff33165f5260205260ff60405f20541615610ebd565b503373ffffffffffffffffffffffffffffffffffffffff82161415610eb7565b346101895760207ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc360112610189576004356110228161170b565b505f526004602052602073ffffffffffffffffffffffffffffffffffffffff60405f205416604051908152f35b34610189575f7ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc360112610189576040515f5f5461108c8161143c565b8084529060018116908115610c1057506001146110b3576107d7836107c38185038261138b565b5f8080527f290decd9548b62a8d60345a988386fc84ba6bc95484008f6362f93160ef3e563939250905b8082106110f5575090915081016020016107c3610ba2565b9192600181602092548385880101520191019092916110dd565b346101895760207ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc36011261018957600435907fffffffff00000000000000000000000000000000000000000000000000000000821680920361018957817fad092b5c00000000000000000000000000000000000000000000000000000000602093149081156111a1575b5015158152f35b7f49064906000000000000000000000000000000000000000000000000000000008114915081156111d4575b508361119a565b7f80ac58cd00000000000000000000000000000000000000000000000000000000811491508115611238575b811561120e575b50836111cd565b7f01ffc9a70000000000000000000000000000000000000000000000000000000091501483611207565b7f5b5e139f0000000000000000000000000000000000000000000000000000000081149150611200565b907fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe0601f602080948051918291828752018686015e5f8582860101520116010190565b6004359073ffffffffffffffffffffffffffffffffffffffff8216820361018957565b6024359073ffffffffffffffffffffffffffffffffffffffff8216820361018957565b7ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc60609101126101895760043573ffffffffffffffffffffffffffffffffffffffff81168103610189579060243573ffffffffffffffffffffffffffffffffffffffff81168103610189579060443590565b9181601f840112156101895782359167ffffffffffffffff8311610189576020838186019501011161018957565b90601f7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe0910116810190811067ffffffffffffffff8211176102f557604052565b67ffffffffffffffff81116102f557601f017fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe01660200190565b929192611412826113cc565b91611420604051938461138b565b829481845281830111610189578281602093845f960137010152565b90600182811c92168015611483575b602083101461145657565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52602260045260245ffd5b91607f169161144b565b919073ffffffffffffffffffffffffffffffffffffffff1691821561072d5773ffffffffffffffffffffffffffffffffffffffff90825f5260026020528160405f2054169333611658575b83851515806115ea575b825f52600360205260405f2060018154019055815f52600260205260405f20837fffffffffffffffffffffffff000000000000000000000000000000000000000082541617905586604051938381837fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef5f80a4826115df575b50506115a2575b50501680830361157157505050565b7f64283d7b000000000000000000000000000000000000000000000000000000005f5260045260245260445260645ffd5b7f4e06b4e7000e659094299b3533b47b6aa8ad048e95e872d23d1f4ee55af89cfe60205f9383855260088252846040812055848152a35f83611562565b14159050865f61155b565b611621825f52600460205260405f207fffffffffffffffffffffffff00000000000000000000000000000000000000008154169055565b865f52600360205260405f207fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff81540190556114e2565b6116638433876118df565b6114d8565b67ffffffffffffffff81116102f55760051b60200190565b80518210156116945760209160051b010190565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b805f52600860205267ffffffffffffffff60405f205460a01c164211611706575f52600860205273ffffffffffffffffffffffffffffffffffffffff60405f20541690565b505f90565b805f52600260205273ffffffffffffffffffffffffffffffffffffffff60405f205416908115611739575090565b7f7e273289000000000000000000000000000000000000000000000000000000005f5260045260245ffd5b805f52600260205273ffffffffffffffffffffffffffffffffffffffff60405f205416918173ffffffffffffffffffffffffffffffffffffffff82166118ce575b50508115159081611860575b805f52600260205260405f207fffffffffffffffffffffffff0000000000000000000000000000000000000000815416905560405191815f857fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef8280a48080611858575b5061181f57505090565b7f4e06b4e7000e659094299b3533b47b6aa8ad048e95e872d23d1f4ee55af89cfe60205f9383855260088252846040812055848152a390565b90505f611815565b611897815f52600460205260405f207fffffffffffffffffffffffff00000000000000000000000000000000000000008154169055565b825f52600360205260405f207fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff81540190556117b1565b6118d891846118df565b5f816117a5565b9073ffffffffffffffffffffffffffffffffffffffff16908115158061197e575b1561190a57505050565b73ffffffffffffffffffffffffffffffffffffffff1661195057507f7e273289000000000000000000000000000000000000000000000000000000005f5260045260245ffd5b7f177e802f000000000000000000000000000000000000000000000000000000005f5260045260245260445ffd5b5073ffffffffffffffffffffffffffffffffffffffff81168281149081156119d2575b50806119005750825f5260046020528173ffffffffffffffffffffffffffffffffffffffff60405f20541614611900565b90505f52600560205260405f20825f5260205260ff60405f2054165f6119a1565b919091805f52600660205260405f20835167ffffffffffffffff81116102f557611a1d825461143c565b601f8111611b70575b506020601f8211600114611aab57908060209493927ff8e1a15aba9398e019f0b49df1a4fde98ee17ae345cb5f6b5e2c27f5033e8ce796975f92611aa0575b50507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8260011b9260031b1c19161790555b604051908152a1565b015190505f80611a65565b7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe0821695835f52815f20965f5b818110611b58575096600192849260209796957ff8e1a15aba9398e019f0b49df1a4fde98ee17ae345cb5f6b5e2c27f5033e8ce7999a10611b21575b505050811b019055611a97565b01517fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff60f88460031b161c191690555f8080611b14565b83830151895560019098019760209384019301611ad8565b825f5260205f20601f830160051c81019160208410611bab575b601f0160051c01905b818110611ba05750611a26565b5f8155600101611b93565b9091508190611b8a565b93909293823b611bc7575b5050505050565b611c3473ffffffffffffffffffffffffffffffffffffffff928360209516968460405197889687967f150b7a020000000000000000000000000000000000000000000000000000000088521660048701521660248501526044840152608060648401526084830190611262565b03815f865af15f9181611d2f575b50611cb057503d15611ca9573d611c58816113cc565b90611c66604051928361138b565b81523d5f602083013e5b80519081611ca457827f64a0ae92000000000000000000000000000000000000000000000000000000005f5260045260245ffd5b602001fd5b6060611c70565b7fffffffff000000000000000000000000000000000000000000000000000000007f150b7a0200000000000000000000000000000000000000000000000000000000911603611d0457505f80808080611bc0565b7f64a0ae92000000000000000000000000000000000000000000000000000000005f5260045260245ffd5b9091506020813d602011611d84575b81611d4b6020938361138b565b8101031261018957517fffffffff000000000000000000000000000000000000000000000000000000008116810361018957905f611c42565b3d9150611d3e565b611d958161170b565b505f52600660205260405f2060405190815f825492611db38461143c565b8084529360018116908115611e2f5750600114611deb575b50611dd89250038261138b565b5f604051611de760208261138b565b5290565b90505f9291925260205f20905f915b818310611e13575050906020611dd8928201015f611dcb565b6020919350806001915483858801015201910190918392611dfa565b60209350611dd89592507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff0091501682840152151560051b8201015f611dcb56fea2646970667358221220adc52842e4f195e4997ed4a2ecad042f69415c6143e949fffe336119dbb10b4964736f6c634300081c0033" as const;

// Arachnid's Deterministic Deployment Proxy address
export const DDP_DEPLOYER = "0x4e59b44847b379578588920cA78FbF26c0B4956C" as const;

// Default salt for consistent deployment addresses across chains
export const DEFAULT_SALT =
  "0x0000000000000000000000000000000000000000000000000000000000000000" as const;
