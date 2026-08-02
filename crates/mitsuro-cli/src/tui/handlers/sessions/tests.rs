use super::storage_role_to_api_role;
use crate::ai::types::Role;

#[test]
fn storage_role_mapping_preserves_tool_role() {
    assert_eq!(storage_role_to_api_role("tool"), Role::Tool);
    assert_eq!(storage_role_to_api_role("assistant"), Role::Assistant);
}
