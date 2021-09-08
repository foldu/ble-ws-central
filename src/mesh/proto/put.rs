use super::ModelIdentifier;

pub(super) fn put_model_id(mut buf: impl bytes::BufMut, id: super::ModelIdentifier) {
    match id {
        ModelIdentifier::Sig(id) => {
            buf.put_u16(id);
        }
        ModelIdentifier::Vendor {
            company_id,
            model_id,
        } => {
            buf.put_u16_le(company_id);
            buf.put_u16_le(model_id);
        }
    }
}
