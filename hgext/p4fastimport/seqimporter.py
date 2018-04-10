# (c) 2017-present Facebook Inc.
from __future__ import absolute_import

import re

from mercurial.i18n import _
from mercurial import (
    context,
)

from . import importer, lfs, p4

class ChangelistImporter(object):
    def __init__(self, ui, repo, ctx, client, storepath):
        self.ui = ui
        self.repo = repo
        self.node = self.repo[ctx].node()
        self.client = client
        self.storepath = storepath

    def importcl(self, p4cl, bookmark=None):
        try:
            ctx, largefiles = self._import(p4cl)
            self.node = self.repo[ctx].node()
            return ctx, largefiles
        except Exception as e:
            self.ui.write_err(_('Failed importing CL%d: %s\n') % (p4cl.cl, e))
            raise

    def _import(self, p4cl):
        '''Converts the provided p4 CL into a commit in hg.
        Returns a tuple containing hg node and largefiles for new commit'''
        self.ui.debug('importing CL%d\n' % p4cl.cl)
        fstat = p4.parse_fstat(p4cl.cl, self.client)
        added_or_modified = []
        removed = set()
        p4flogs = {}
        for info in fstat:
            action = info['action']
            p4path = info['depotFile']
            data = {p4cl.cl: {'action': action, 'type': info['type']}}
            hgpath = importer.relpath(self.client, p4path)
            p4flogs[hgpath] = p4.P4Filelog(p4path, data)

            if action in p4.ACTION_DELETE + p4.ACTION_ARCHIVE:
                removed.add(hgpath)
            else:
                added_or_modified.append((p4path, hgpath))

        moved = self._get_move_info(p4cl, p4flogs)
        node = self._create_commit(p4cl, p4flogs, removed, moved)
        largefiles = self._get_largefiles(p4cl, added_or_modified, node)

        return node, largefiles

    def _get_largefiles(self, p4cl, files, node):
        largefiles = []
        ctx = self.repo[node]
        for p4path, hgpath in files:
            flog = self.repo.file(hgpath)
            fnode = ctx.filenode(hgpath)
            islfs, oid = lfs.getlfsinfo(flog, fnode)
            if islfs:
                largefiles.append((p4cl.cl, p4path, oid))
                self.ui.debug('largefile: %s, oid: %s\n' % (hgpath, oid))
        return largefiles

    def _get_move_info(self, p4cl, p4flogs):
        '''Returns a dict where entries are (dst, src)'''
        moves = {}
        files_in_clientspec = {
            p4flog._depotfile: hgpath for hgpath, p4flog in p4flogs.items()
        }
        for filename, info in p4cl.parsed['files'].items():
            if filename not in files_in_clientspec:
                continue
            src = info.get('src')
            if src:
                hgdst = files_in_clientspec[filename]
                # The below could return None if the source of the move is
                # outside of client view. That is expected.
                # This info will be used when creating the commit, and value of
                # None in the moves dictionary is a no-op, it will treat it as
                # an add in hg. As it just came into the client view we cannot
                # store any move info for it in hg (even though it was a legit
                # move in perforce).
                hgsrc = importer.relpath(
                    self.client,
                    src,
                    ignore_nonexisting=True,
                )
                moves[hgdst] = hgsrc
        return moves

    def _create_commit(self, p4cl, p4flogs, removed, moved):
        '''Uses a memory context to commit files into the repo'''
        def getfile(repo, memctx, path):
            if path in removed:
                # A path that shows up in files (below) but returns None in this
                # function implies a deletion.
                return None

            p4flog = p4flogs[path]
            data, src = importer.get_p4_file_content(
                self.storepath,
                p4flog,
                p4cl,
            )
            self.ui.debug('file: %s, src: %s\n' % (p4flog._depotfile, src))
            islink = p4flog.issymlink(p4cl.cl)
            if islink:
                # p4 will give us content with a trailing newline, symlinks
                # cannot end with newline
                data = data.rstrip()
            if p4flog.iskeyworded(p4cl.cl):
                data = re.sub(importer.KEYWORD_REGEX, r'$\1$', data)

            return context.memfilectx(
                repo,
                memctx,
                path,
                data,
                islink=islink,
                isexec=p4flog.isexec(p4cl.cl),
                copied=moved.get(path),
            )

        return context.memctx(
            self.repo,                        # repository
            (self.node, None),                # parents
            p4cl.description,                 # commit message
            p4flogs.keys(),                   # files affected by this change
            getfile,                          # fn - see above
            user=p4cl.user,                   # commit author
            date=p4cl.hgdate,                 # commit date
            extra={'p4changelist': p4cl.cl},  # commit extras
        ).commit()
