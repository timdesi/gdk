#ifndef GDK_TRANSACTION_UTILS_HPP
#define GDK_TRANSACTION_UTILS_HPP
#pragma once

#include <array>
#include <memory>
#include <utility>

#include "amount.hpp"

namespace ga {
namespace sdk {
    class ga_pubkeys;
    class session_impl;
    class user_pubkeys;

    enum class script_type : int {
        // Script types returned by the Green backend server
        ga_pubkey_hash_out = 2, // Not actually generated by the server, used for sweeping
        ga_p2sh_fortified_out = 10,
        ga_p2sh_p2wsh_fortified_out = 14,
        ga_p2sh_p2wsh_csv_fortified_out = 15,
        ga_redeem_p2sh_fortified = 150,
        ga_redeem_p2sh_p2wsh_fortified = 159,
        ga_redeem_p2sh_p2wsh_csv_fortified = 162
    };

    namespace address_type {
        extern const std::string p2pkh; // Not generated by the server, used for sweeping
        extern const std::string p2wpkh;
        extern const std::string p2sh_p2wpkh;
        extern const std::string p2sh;
        extern const std::string p2wsh; // Actually p2sh-p2wsh
        extern const std::string csv;
    }; // namespace address_type

    const uint32_t NO_CHANGE_INDEX = 0xffffffff;

    extern const std::string FAKE_BLINDING_KEY;

    bool is_segwit_address_type(const nlohmann::json& utxo);

    std::string get_address_from_public_key(
        const network_parameters& net_params, byte_span_t public_key, const std::string& addr_type);

    std::string get_address_from_script(
        const network_parameters& net_params, byte_span_t script, const std::string& addr_type);

    std::string get_address_from_scriptpubkey(const network_parameters& net_params, byte_span_t scriptpubkey);

    std::vector<unsigned char> output_script_from_utxo(const network_parameters& net_params, ga_pubkeys& pubkeys,
        user_pubkeys& usr_pubkeys, user_pubkeys& recovery_pubkeys, const nlohmann::json& utxo);

    // Returns the 32 byte asset id in hex, or "btc" for bitcoin
    std::string asset_id_from_json(const network_parameters& net_params, const nlohmann::json& json);

    // Make a multisig scriptSig
    std::vector<unsigned char> input_script(bool low_r, const std::vector<unsigned char>& prevout_script,
        const ecdsa_sig_t& user_sig, const ecdsa_sig_t& ga_sig, uint32_t user_sighash, uint32_t ga_sighash);

    // Make a multisig scriptSig with a user signature and PUSH(0) marker for the GA sig
    std::vector<unsigned char> input_script(bool low_r, const std::vector<unsigned char>& prevout_script,
        const ecdsa_sig_t& user_sig, uint32_t user_sighash);

    // Make a multisig scriptSig with dummy signatures for (fee estimation)
    std::vector<unsigned char> dummy_input_script(bool low_r, const std::vector<unsigned char>& prevout_script);

    std::vector<unsigned char> dummy_external_input_script(bool low_r, byte_span_t pub_key);

    std::vector<unsigned char> witness_script(byte_span_t script, uint32_t witness_ver);

    // Compute the fee for a tx
    amount get_tx_fee(const wally_tx_ptr& tx, amount min_fee_rate, amount fee_rate);

    // Get scriptpubkey from address (address is expected to be valid)
    std::vector<unsigned char> scriptpubkey_from_address(
        const network_parameters& net_params, const std::string& address, bool confidential = true);

    // Set the error in a transaction, if it hasn't been set already
    void set_tx_error(nlohmann::json& result, const std::string& error);

    // Add an output to a tx given its address
    amount add_tx_output(const network_parameters& net_params, nlohmann::json& result, wally_tx_ptr& tx,
        const std::string& address, amount::value_type satoshi = 0, const std::string& asset_id = {});

    // Add a fee output to a tx, returns the index in tx->outputs
    size_t add_tx_fee_output(const network_parameters& net_params, wally_tx_ptr& tx, amount::value_type satoshi);

    void set_tx_output_commitment(
        wally_tx_ptr& tx, uint32_t index, const std::string& asset_id, amount::value_type satoshi);

    std::string validate_tx_addressee(
        const network_parameters& net_params, nlohmann::json& result, nlohmann::json& addressee);

    // Add an output from a JSON addressee
    amount add_tx_addressee(session_impl& session, const network_parameters& net_params, nlohmann::json& result,
        wally_tx_ptr& tx, nlohmann::json& addressee);

    vbf_t generate_final_vbf(byte_span_t input_abfs, byte_span_t input_vbfs, uint64_span_t input_values,
        const std::vector<abf_t>& output_abfs, const std::vector<vbf_t>& output_vbfs, uint32_t num_inputs);

    // Update the json tx size/fee rate information from tx
    void update_tx_size_info(const network_parameters& net_params, const wally_tx_ptr& tx, nlohmann::json& result);

    // Update the json tx representation with info from tx
    void update_tx_info(session_impl& session, const wally_tx_ptr& tx, nlohmann::json& result);

    // Compute the subaccounts a tx uses from its inputs
    std::set<uint32_t> get_tx_subaccounts(const nlohmann::json& details);

    // Return the single subaccount in subaccounts or throw an error
    uint32_t get_single_subaccount(const std::set<uint32_t>& subaccounts);

    // Returns true if a tx has AMP inputs
    bool tx_has_amp_inputs(session_impl& session, const nlohmann::json& details);

    // Set the locktime on tx to avoid fee sniping
    void set_anti_snipe_locktime(const wally_tx_ptr& tx, uint32_t current_block_height);
} // namespace sdk
} // namespace ga

#endif
