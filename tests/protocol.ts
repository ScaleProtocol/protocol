import * as anchor from "@project-serum/anchor";
import { Program } from "@project-serum/anchor";
import { ScaleProtocol } from "../target/types/scale_protocol";

describe("scale-protocol", () => {
  // Configure the client to use the local cluster.
  anchor.setProvider(anchor.AnchorProvider.env());

  const program = anchor.workspace.ScaleProtocol as Program<ScaleProtocol>;

  it("Is initialized!", async () => {
    // Add your test here.
    const tx = await program.methods.initialize().rpc();
    console.log("Your transaction signature", tx);
  });
});
