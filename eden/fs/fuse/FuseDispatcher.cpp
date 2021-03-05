/*
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * This software may be used and distributed according to the terms of the
 * GNU General Public License version 2.
 */

#ifndef _WIN32

#include "eden/fs/fuse/FuseDispatcher.h"

#include <folly/Exception.h>
#include <folly/Format.h>
#include <folly/executors/GlobalExecutor.h>
#include <folly/futures/Future.h>
#include <folly/logging/xlog.h>

#include "eden/fs/fuse/DirList.h"
#include "eden/fs/utils/StatTimes.h"

using namespace folly;

namespace facebook {
namespace eden {

FuseDispatcher::Attr::Attr(const struct stat& st, uint64_t timeout)
    : st(st), timeout_seconds(timeout) {}

fuse_attr_out FuseDispatcher::Attr::asFuseAttr() const {
  // Ensure that we initialize the members to zeroes;
  // This is important on macOS where there are a couple
  // of additional fields (notably `flags`) that influence
  // file accessibility.
  fuse_attr_out result{};

  result.attr.ino = st.st_ino;
  result.attr.size = st.st_size;
  result.attr.blocks = st.st_blocks;
  result.attr.atime = st.st_atime;
  result.attr.atimensec = stAtime(st).tv_nsec;
  result.attr.mtime = st.st_mtime;
  result.attr.mtimensec = stMtime(st).tv_nsec;
  result.attr.ctime = st.st_ctime;
  result.attr.ctimensec = stCtime(st).tv_nsec;
  result.attr.mode = st.st_mode;
  result.attr.nlink = st.st_nlink;
  result.attr.uid = st.st_uid;
  result.attr.gid = st.st_gid;
  result.attr.rdev = st.st_rdev;
  result.attr.blksize = st.st_blksize;

  result.attr_valid_nsec = 0;
  result.attr_valid = timeout_seconds;

  return result;
}

FuseDispatcher::~FuseDispatcher() {}

FuseDispatcher::FuseDispatcher(EdenStats* stats) : stats_(stats) {}

void FuseDispatcher::initConnection(const fuse_init_out& out) {
  connInfo_ = out;
}

void FuseDispatcher::destroy() {}

folly::Future<fuse_entry_out> FuseDispatcher::lookup(
    uint64_t /*requestID*/,
    InodeNumber /*parent*/,
    PathComponentPiece /*name*/,
    ObjectFetchContext& /*context*/) {
  throwSystemErrorExplicit(ENOENT);
}

void FuseDispatcher::forget(InodeNumber /*ino*/, unsigned long /*nlookup*/) {}

folly::Future<FuseDispatcher::Attr> FuseDispatcher::getattr(
    InodeNumber /*ino*/,
    ObjectFetchContext& /*context*/) {
  throwSystemErrorExplicit(ENOENT);
}

folly::Future<FuseDispatcher::Attr> FuseDispatcher::setattr(
    InodeNumber /*ino*/,
    const fuse_setattr_in& /*attr*/
) {
  FUSELL_NOT_IMPL();
}

folly::Future<std::string> FuseDispatcher::readlink(
    InodeNumber /*ino*/,
    bool /*kernelCachesReadlink*/) {
  FUSELL_NOT_IMPL();
}

folly::Future<fuse_entry_out> FuseDispatcher::mknod(
    InodeNumber /*parent*/,
    PathComponentPiece /*name*/,
    mode_t /*mode*/,
    dev_t /*rdev*/) {
  FUSELL_NOT_IMPL();
}

folly::Future<fuse_entry_out>
FuseDispatcher::mkdir(InodeNumber, PathComponentPiece, mode_t) {
  FUSELL_NOT_IMPL();
}

folly::Future<folly::Unit>
FuseDispatcher::unlink(InodeNumber, PathComponentPiece, ObjectFetchContext&) {
  FUSELL_NOT_IMPL();
}

folly::Future<folly::Unit>
FuseDispatcher::rmdir(InodeNumber, PathComponentPiece, ObjectFetchContext&) {
  FUSELL_NOT_IMPL();
}

folly::Future<fuse_entry_out>
FuseDispatcher::symlink(InodeNumber, PathComponentPiece, folly::StringPiece) {
  FUSELL_NOT_IMPL();
}

folly::Future<folly::Unit> FuseDispatcher::rename(
    InodeNumber,
    PathComponentPiece,
    InodeNumber,
    PathComponentPiece) {
  FUSELL_NOT_IMPL();
}

folly::Future<fuse_entry_out>
FuseDispatcher::link(InodeNumber, InodeNumber, PathComponentPiece) {
  FUSELL_NOT_IMPL();
}

folly::Future<uint64_t> FuseDispatcher::open(
    InodeNumber /*ino*/,
    int /*flags*/) {
  FUSELL_NOT_IMPL();
}

folly::Future<folly::Unit> FuseDispatcher::release(
    InodeNumber /*ino*/,
    uint64_t /*fh*/) {
  FUSELL_NOT_IMPL();
}

folly::Future<uint64_t> FuseDispatcher::opendir(
    InodeNumber /*ino*/,
    int /*flags*/) {
  FUSELL_NOT_IMPL();
}

folly::Future<folly::Unit> FuseDispatcher::releasedir(
    InodeNumber /*ino*/,
    uint64_t /*fh*/) {
  FUSELL_NOT_IMPL();
}

folly::Future<BufVec> FuseDispatcher::read(
    InodeNumber /*ino*/,
    size_t /*size*/,
    off_t /*off*/,
    ObjectFetchContext& /*context*/) {
  FUSELL_NOT_IMPL();
}

folly::Future<size_t> FuseDispatcher::write(
    InodeNumber /*ino*/,
    StringPiece /*data*/,
    off_t /*off*/) {
  FUSELL_NOT_IMPL();
}

folly::Future<folly::Unit> FuseDispatcher::flush(InodeNumber, uint64_t) {
  FUSELL_NOT_IMPL();
}

folly::Future<folly::Unit>
FuseDispatcher::fallocate(InodeNumber, uint64_t, uint64_t) {
  FUSELL_NOT_IMPL();
}

folly::Future<folly::Unit> FuseDispatcher::fsync(InodeNumber, bool) {
  FUSELL_NOT_IMPL();
}

folly::Future<folly::Unit> FuseDispatcher::fsyncdir(InodeNumber, bool) {
  FUSELL_NOT_IMPL();
}

folly::Future<DirList> FuseDispatcher::readdir(
    InodeNumber,
    DirList&&,
    off_t,
    uint64_t,
    ObjectFetchContext&) {
  FUSELL_NOT_IMPL();
}

folly::Future<struct fuse_kstatfs> FuseDispatcher::statfs(InodeNumber /*ino*/) {
  struct fuse_kstatfs info = {};

  // Suggest a large blocksize to software that looks at that kind of thing
  // bsize will be returned to applications that call pathconf() with
  // _PC_REC_MIN_XFER_SIZE
  info.bsize = getConnInfo().max_readahead;

  // The fragment size is returned as the _PC_REC_XFER_ALIGN and
  // _PC_ALLOC_SIZE_MIN pathconf() settings.
  // 4096 is commonly used by many filesystem types.
  info.frsize = 4096;

  // Ensure that namelen is set to a non-zero value.
  // The value we return here will be visible to programs that call pathconf()
  // with _PC_NAME_MAX.  Returning 0 will confuse programs that try to honor
  // this value.
  info.namelen = 255;

  return info;
}

folly::Future<folly::Unit> FuseDispatcher::setxattr(
    InodeNumber /*ino*/,
    folly::StringPiece /*name*/,
    folly::StringPiece /*value*/,
    int /*flags*/) {
  FUSELL_NOT_IMPL();
}

const int FuseDispatcher::kENOATTR =
#ifndef ENOATTR
    ENODATA // Linux
#else
    ENOATTR
#endif
    ;

folly::Future<std::string> FuseDispatcher::getxattr(
    InodeNumber /*ino*/,
    folly::StringPiece /*name*/) {
  throwSystemErrorExplicit(kENOATTR);
}

folly::Future<std::vector<std::string>> FuseDispatcher::listxattr(
    InodeNumber /*ino*/) {
  return std::vector<std::string>();
}

folly::Future<folly::Unit> FuseDispatcher::removexattr(
    InodeNumber /*ino*/,
    folly::StringPiece /*name*/) {
  FUSELL_NOT_IMPL();
}

folly::Future<folly::Unit> FuseDispatcher::access(
    InodeNumber /*ino*/,
    int /*mask*/) {
  // Note that if you mount with the "default_permissions" kernel mount option,
  // the kernel will perform all permissions checks for you, and will never
  // invoke access() directly.
  //
  // Implementing access() is only needed when not using the
  // "default_permissions" option.
  FUSELL_NOT_IMPL();
}

folly::Future<fuse_entry_out>
FuseDispatcher::create(InodeNumber, PathComponentPiece, mode_t, int) {
  FUSELL_NOT_IMPL();
}

folly::Future<uint64_t> FuseDispatcher::bmap(
    InodeNumber /*ino*/,
    size_t /*blocksize*/,
    uint64_t /*idx*/) {
  FUSELL_NOT_IMPL();
}

const fuse_init_out& FuseDispatcher::getConnInfo() const {
  return connInfo_;
}

EdenStats* FuseDispatcher::getStats() const {
  return stats_;
}

} // namespace eden
} // namespace facebook

#endif
